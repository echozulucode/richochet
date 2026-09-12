import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { StoreApi } from 'zustand/vanilla';

import {
  __resetUpdaterClient,
  __setUpdaterClientForTests,
  createUpdateStore,
  hasPendingUpdate,
} from './updateStore';
import type {
  UpdateDownloadEvent,
  UpdateHandle,
  UpdateState,
  UpdateStore,
  UpdaterClient,
} from './updateStore';

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (error: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** A pending update whose download is driven by hand: emit events, then finish or fail. */
function fakeHandle(version = '0.1.1') {
  let emit: ((event: UpdateDownloadEvent) => void) | null = null;
  const done = deferred<void>();
  const handle: UpdateHandle = {
    version,
    notes: 'Bug fixes.',
    download: vi.fn((onEvent: (event: UpdateDownloadEvent) => void) => {
      emit = onEvent;
      return done.promise;
    }),
    install: vi.fn(() => Promise.resolve()),
  };
  return {
    handle,
    emit(event: UpdateDownloadEvent) {
      if (!emit) throw new Error('download has not started');
      emit(event);
    },
    finish: () => done.resolve(),
    fail: (error: unknown) => done.reject(error),
  };
}

function fakeClient(overrides: Partial<UpdaterClient> = {}): UpdaterClient {
  return {
    getVersion: vi.fn(() => Promise.resolve('0.1.0')),
    check: vi.fn(() => Promise.resolve(null)),
    relaunch: vi.fn(() => Promise.resolve()),
    ...overrides,
  };
}

describe('updateStore state machine', () => {
  let store: StoreApi<UpdateStore>;
  let warn: ReturnType<typeof vi.spyOn>;

  const kind = (): UpdateState['kind'] => store.getState().state.kind;

  /** Run a check that finds `handle`, leaving the store in `available`. */
  async function reachAvailable(handle: UpdateHandle): Promise<void> {
    __setUpdaterClientForTests(fakeClient({ check: () => Promise.resolve(handle) }));
    await store.getState().checkForUpdates();
    expect(kind()).toBe('available');
  }

  beforeEach(() => {
    warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    store = createUpdateStore();
  });

  afterEach(() => {
    __resetUpdaterClient();
  });

  it('starts idle, with no version and nothing checked', () => {
    expect(store.getState().state).toEqual({ kind: 'idle' });
    expect(store.getState().currentVersion).toBeNull();
    expect(store.getState().hasChecked).toBe(false);
  });

  it('idle -> checking -> up-to-date when the check finds nothing', async () => {
    const pending = deferred<UpdateHandle | null>();
    __setUpdaterClientForTests(fakeClient({ check: () => pending.promise }));

    const run = store.getState().checkForUpdates();
    expect(kind()).toBe('checking');
    expect(store.getState().hasChecked).toBe(true);

    pending.resolve(null);
    await run;
    expect(store.getState().state).toEqual({ kind: 'up-to-date' });
    expect(store.getState().currentVersion).toBe('0.1.0');
    expect(warn).not.toHaveBeenCalled();
  });

  it('idle -> checking -> available when the check finds an update', async () => {
    const { handle } = fakeHandle('0.2.0');
    __setUpdaterClientForTests(fakeClient({ check: () => Promise.resolve(handle) }));

    await store.getState().checkForUpdates();
    expect(store.getState().state).toEqual({
      kind: 'available',
      version: '0.2.0',
      notes: 'Bug fixes.',
    });
  });

  it('checking -> error on a failed check, with exactly one warning and no throw', async () => {
    __setUpdaterClientForTests(
      fakeClient({ check: () => Promise.reject(new Error('getaddrinfo ENOTFOUND github.com')) }),
    );

    await expect(store.getState().checkForUpdates()).resolves.toBeUndefined();
    expect(store.getState().state).toEqual({ kind: 'error' });
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0]?.[0])).toContain('ENOTFOUND');
    expect(hasPendingUpdate(store.getState().state)).toBe(false);
  });

  it('a failed version read is not a failed check', async () => {
    __setUpdaterClientForTests(
      fakeClient({ getVersion: () => Promise.reject(new Error('no Tauri runtime')) }),
    );

    await store.getState().checkForUpdates();
    expect(kind()).toBe('up-to-date');
    expect(store.getState().currentVersion).toBeNull();
    expect(warn).not.toHaveBeenCalled();
  });

  it('with no Tauri at all (the ?backend=mock path) both fail silently down to error', async () => {
    const noTauri = () => Promise.reject(new TypeError('Cannot read properties of undefined'));
    __setUpdaterClientForTests({ getVersion: noTauri, check: noTauri, relaunch: noTauri });

    await store.getState().checkOnLaunch();
    expect(kind()).toBe('error');
    expect(store.getState().currentVersion).toBeNull();
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it('checkOnLaunch runs the check once, however many times it is called', async () => {
    const client = fakeClient();
    __setUpdaterClientForTests(client);

    await Promise.all([store.getState().checkOnLaunch(), store.getState().checkOnLaunch()]);
    await store.getState().checkOnLaunch();
    expect(client.check).toHaveBeenCalledTimes(1);
  });

  it('ignores a second checkForUpdates while one is in flight', async () => {
    const pending = deferred<UpdateHandle | null>();
    const client = fakeClient({ check: vi.fn(() => pending.promise) });
    __setUpdaterClientForTests(client);

    const first = store.getState().checkForUpdates();
    await store.getState().checkForUpdates();
    pending.resolve(null);
    await first;
    expect(client.check).toHaveBeenCalledTimes(1);
  });

  it('available -> downloading -> ready-to-install, with a percentage when the length is known', async () => {
    const fake = fakeHandle();
    await reachAvailable(fake.handle);

    const run = store.getState().startDownload();
    expect(store.getState().state).toEqual({
      kind: 'downloading',
      version: '0.1.1',
      percent: null,
    });

    fake.emit({ event: 'Started', data: { contentLength: 1000 } });
    expect(store.getState().state).toEqual({ kind: 'downloading', version: '0.1.1', percent: 0 });

    fake.emit({ event: 'Progress', data: { chunkLength: 470 } });
    expect(store.getState().state).toEqual({ kind: 'downloading', version: '0.1.1', percent: 47 });

    fake.emit({ event: 'Progress', data: { chunkLength: 530 } });
    expect(store.getState().state).toMatchObject({ kind: 'downloading', percent: 100 });

    fake.emit({ event: 'Finished' });
    // Finished alone is not "done" - the download promise is the authority.
    expect(kind()).toBe('downloading');

    fake.finish();
    await run;
    expect(store.getState().state).toEqual({ kind: 'ready-to-install', version: '0.1.1' });
    expect(fake.handle.install).not.toHaveBeenCalled();
  });

  it('never reports more than 100% when the server under-declares the length', async () => {
    const fake = fakeHandle();
    await reachAvailable(fake.handle);

    const run = store.getState().startDownload();
    fake.emit({ event: 'Started', data: { contentLength: 100 } });
    fake.emit({ event: 'Progress', data: { chunkLength: 250 } });
    expect(store.getState().state).toMatchObject({ percent: 100 });
    fake.finish();
    await run;
  });

  it('downloads with no percentage when the manifest gave no content length', async () => {
    const fake = fakeHandle();
    await reachAvailable(fake.handle);

    const run = store.getState().startDownload();
    fake.emit({ event: 'Started', data: {} });
    expect(store.getState().state).toEqual({
      kind: 'downloading',
      version: '0.1.1',
      percent: null,
    });

    fake.emit({ event: 'Progress', data: { chunkLength: 4096 } });
    expect(store.getState().state).toEqual({
      kind: 'downloading',
      version: '0.1.1',
      percent: null,
    });

    fake.finish();
    await run;
    expect(kind()).toBe('ready-to-install');
  });

  it('downloading -> error on a failed download, with one warning and no throw', async () => {
    const fake = fakeHandle();
    await reachAvailable(fake.handle);

    const run = store.getState().startDownload();
    fake.emit({ event: 'Started', data: { contentLength: 1000 } });
    fake.emit({ event: 'Progress', data: { chunkLength: 100 } });
    fake.fail(new Error('connection reset'));

    await expect(run).resolves.toBeUndefined();
    expect(store.getState().state).toEqual({ kind: 'error' });
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0]?.[0])).toContain('connection reset');
  });

  it('startDownload does nothing unless an update is available', async () => {
    await store.getState().startDownload();
    expect(kind()).toBe('idle');

    __setUpdaterClientForTests(fakeClient());
    await store.getState().checkForUpdates();
    await store.getState().startDownload();
    expect(kind()).toBe('up-to-date');
  });

  it('ready-to-install -> restart installs and then relaunches exactly once', async () => {
    const fake = fakeHandle();
    const order: string[] = [];
    fake.handle.install = vi.fn(() => {
      order.push('install');
      return Promise.resolve();
    });
    const relaunch = vi.fn(() => {
      order.push('relaunch');
      return Promise.resolve();
    });
    __setUpdaterClientForTests(fakeClient({ check: () => Promise.resolve(fake.handle), relaunch }));

    await store.getState().checkForUpdates();
    const run = store.getState().startDownload();
    fake.finish();
    await run;

    // A double click must not install or relaunch twice.
    await Promise.all([store.getState().restart(), store.getState().restart()]);

    expect(relaunch).toHaveBeenCalledTimes(1);
    expect(fake.handle.install).toHaveBeenCalledTimes(1);
    expect(order).toEqual(['install', 'relaunch']);
  });

  it('restart does nothing before the download has finished', async () => {
    const fake = fakeHandle();
    const client = fakeClient({ check: () => Promise.resolve(fake.handle) });
    __setUpdaterClientForTests(client);
    await store.getState().checkForUpdates();

    await store.getState().restart();
    const run = store.getState().startDownload();
    await store.getState().restart();
    fake.finish();
    await run;

    expect(client.relaunch).not.toHaveBeenCalled();
    expect(fake.handle.install).not.toHaveBeenCalled();
  });

  it('ready-to-install -> error when the relaunch fails', async () => {
    const fake = fakeHandle();
    __setUpdaterClientForTests(
      fakeClient({
        check: () => Promise.resolve(fake.handle),
        relaunch: () => Promise.reject(new Error('permission denied')),
      }),
    );
    await store.getState().checkForUpdates();
    const run = store.getState().startDownload();
    fake.finish();
    await run;

    await expect(store.getState().restart()).resolves.toBeUndefined();
    expect(kind()).toBe('error');
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it('only available, downloading and ready-to-install light the gear', () => {
    const lit: Record<UpdateState['kind'], boolean> = {
      idle: false,
      checking: false,
      'up-to-date': false,
      error: false,
      available: true,
      downloading: true,
      'ready-to-install': true,
    };
    const samples: UpdateState[] = [
      { kind: 'idle' },
      { kind: 'checking' },
      { kind: 'up-to-date' },
      { kind: 'error' },
      { kind: 'available', version: '1', notes: null },
      { kind: 'downloading', version: '1', percent: null },
      { kind: 'ready-to-install', version: '1' },
    ];
    for (const sample of samples) expect(hasPendingUpdate(sample)).toBe(lit[sample.kind]);
  });
});
