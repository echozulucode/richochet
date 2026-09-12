import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { StoreApi } from 'zustand/vanilla';

/**
 * Update state, and the flow that drives it.
 *
 * ```text
 * idle → checking → up-to-date | available → downloading → ready-to-install
 *                      ↓ (any step)
 *                    error
 * ```
 *
 * Everything here is best-effort. A machine with no network, a GitHub outage or a corporate proxy
 * eating the request must not stop the app from opening, so every failure lands in `error`, which
 * renders exactly the same quiet version line as `idle` — no message, no dot, one `console.warn`
 * for whoever is looking at the log. The next launch checks again.
 */
export type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'up-to-date' }
  | { kind: 'available'; version: string; notes: string | null }
  /** `percent` is null when the manifest declared no content length - show no number, not 0%. */
  | { kind: 'downloading'; version: string; percent: number | null }
  | { kind: 'ready-to-install'; version: string }
  | { kind: 'error' };

/** Download progress, mirroring the updater plugin's channel events. */
export type UpdateDownloadEvent =
  | { event: 'Started'; data: { contentLength?: number } }
  | { event: 'Progress'; data: { chunkLength: number } }
  | { event: 'Finished' };

/** One pending update, as the store needs it. Wraps the plugin's `Update` resource. */
export interface UpdateHandle {
  /** Semver from the manifest, e.g. `"0.1.1"`. */
  version: string;
  /** Release notes from the manifest's `notes` field, if it had any. */
  notes: string | null;
  /** Fetch the package. Resolves when the bytes are on disk, before anything is installed. */
  download: (onEvent: (event: UpdateDownloadEvent) => void) => Promise<void>;
  /**
   * Install the downloaded package.
   *
   * **On Windows this never returns**: the plugin spawns the NSIS installer and then calls
   * `std::process::exit(0)`. That is why the download and the install are two separate steps here
   * rather than one `downloadAndInstall` - a combined call would exit the app the moment the
   * bytes landed, and the `ready-to-install` state the spec asks for would be unreachable.
   */
  install: () => Promise<void>;
}

/**
 * The seam between this store and Tauri.
 *
 * Load-bearing, not a convenience: Vitest runs in jsdom and Playwright in a real browser, and
 * neither has a Tauri runtime. A *static* `import '@tauri-apps/plugin-updater'` would pull the
 * plugin into both bundles, where its first call blows up on a missing `__TAURI_INTERNALS__`.
 * Behind a dynamic `import()` the module is never even fetched unless something asks for it, and
 * when it is, the failure is an ordinary rejected promise we can swallow.
 */
export interface UpdaterClient {
  /** The running app's own version. */
  getVersion: () => Promise<string>;
  /** Ask the update server. Resolves to null when there is nothing newer. */
  check: () => Promise<UpdateHandle | null>;
  /** Quit and come back up. */
  relaunch: () => Promise<void>;
}

const defaultClient: UpdaterClient = {
  async getVersion() {
    const { getVersion } = await import('@tauri-apps/api/app');
    return getVersion();
  },
  async check() {
    const { check } = await import('@tauri-apps/plugin-updater');
    const update = await check();
    if (!update) return null;
    return {
      version: update.version,
      notes: update.body ?? null,
      download: (onEvent) => update.download((event) => onEvent(event)),
      install: () => update.install(),
    };
  },
  async relaunch() {
    const { relaunch } = await import('@tauri-apps/plugin-process');
    await relaunch();
  },
};

let activeClient: UpdaterClient = defaultClient;

/** Test seam - swap the Tauri-backed client for a fake. */
export function __setUpdaterClientForTests(client: UpdaterClient): void {
  activeClient = client;
}

/** Test seam - put the real client back. */
export function __resetUpdaterClient(): void {
  activeClient = defaultClient;
}

export interface UpdateStore {
  state: UpdateState;
  /** The running version, or null when it could not be read (a browser, mostly). */
  currentVersion: string | null;
  /** True once a check has been started, so the launch check runs once across re-renders. */
  hasChecked: boolean;
  /** Read the running version and ask for updates. Never rejects. */
  checkForUpdates: () => Promise<void>;
  /** The launch check: `checkForUpdates`, but at most once per session. Never rejects. */
  checkOnLaunch: () => Promise<void>;
  /** Fetch the available package. Never rejects. */
  startDownload: () => Promise<void>;
  /** Install what was downloaded and restart into it. Never rejects. */
  restart: () => Promise<void>;
}

function warn(step: string, error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  console.warn(`[update] ${step} failed: ${message}`);
}

export function createUpdateStore(): StoreApi<UpdateStore> {
  // The plugin's Update resource lives here rather than in the state union: the UI has no use for
  // it, and keeping it out means every state value stays plain data.
  let handle: UpdateHandle | null = null;
  // `restart` has no state of its own to move through - on Windows the process just ends - so a
  // flag is what stops a double click from installing twice.
  let restarting = false;

  return createStore<UpdateStore>()((set, get) => ({
    state: { kind: 'idle' },
    currentVersion: null,
    hasChecked: false,

    async checkForUpdates() {
      if (get().state.kind === 'checking') return;
      const client = activeClient;
      set({ state: { kind: 'checking' }, hasChecked: true });

      // Two independent requests. The version is only a label, so its failure is not the update
      // flow's failure - outside Tauri it always fails, and the menu simply says "Richochet".
      const version = client
        .getVersion()
        .then((value) => {
          set({ currentVersion: value });
        })
        .catch(() => undefined);

      try {
        const found = await client.check();
        handle = found;
        set(
          found
            ? { state: { kind: 'available', version: found.version, notes: found.notes } }
            : { state: { kind: 'up-to-date' } },
        );
      } catch (error) {
        warn('check', error);
        set({ state: { kind: 'error' } });
      }

      await version;
    },

    async checkOnLaunch() {
      if (get().hasChecked) return;
      await get().checkForUpdates();
    },

    async startDownload() {
      const current = get().state;
      if (current.kind !== 'available' || handle === null) return;
      const { version } = current;
      const pending = handle;

      set({ state: { kind: 'downloading', version, percent: null } });

      let total: number | null = null;
      let received = 0;

      try {
        await pending.download((event) => {
          if (event.event === 'Started') {
            total = event.data.contentLength ?? null;
            received = 0;
          } else if (event.event === 'Progress') {
            received += event.data.chunkLength;
          } else {
            return;
          }
          const percent =
            total === null || total <= 0
              ? null
              : Math.min(100, Math.round((received / total) * 100));
          set({ state: { kind: 'downloading', version, percent } });
        });
        set({ state: { kind: 'ready-to-install', version } });
      } catch (error) {
        warn('download', error);
        set({ state: { kind: 'error' } });
      }
    },

    async restart() {
      const pending = handle;
      if (restarting || get().state.kind !== 'ready-to-install' || pending === null) return;
      restarting = true;
      const client = activeClient;
      try {
        // On Windows this hands off to NSIS and exits the process, so nothing below runs. On
        // macOS and Linux it returns and the app has to bring itself back up.
        await pending.install();
        await client.relaunch();
      } catch (error) {
        warn('restart', error);
        set({ state: { kind: 'error' } });
      } finally {
        restarting = false;
      }
    },
  }));
}

export const updateStore = createUpdateStore();

export function useUpdateStore<T>(selector: (state: UpdateStore) => T): T {
  return useStore(updateStore, selector);
}

/** True while something actionable is pending - the only thing that lights the gear. */
export function hasPendingUpdate(state: UpdateState): boolean {
  return (
    state.kind === 'available' || state.kind === 'downloading' || state.kind === 'ready-to-install'
  );
}
