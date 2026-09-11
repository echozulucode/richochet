import { createMockBackend } from './backends/mock';
import { createTauriBackend } from './backends/tauri';
import { installTestHook, resetTestHook } from './testHook';
import type { BackendName, ConversionBackend } from './types';

/**
 * Backend selection.
 *
 * Tauri is detected by the presence of `window.__TAURI_INTERNALS__`, which the runtime injects
 * before any app code runs. `?backend=mock` forces the browser backend so Playwright can drive
 * the real UI without a desktop shell; `?backend=tauri` forces the other way for completeness.
 */
export function detectBackendName(search: string = globalThis.location?.search ?? ''): BackendName {
  const override = new URLSearchParams(search).get('backend');
  if (override === 'mock' || override === 'tauri') return override;
  const hasTauri =
    typeof window !== 'undefined' &&
    '__TAURI_INTERNALS__' in (window as unknown as Record<string, unknown>);
  return hasTauri ? 'tauri' : 'mock';
}

/** Parse `?latency=200` or `?latency=200,10` into a per-request delay cycle. */
export function parseLatency(search: string = globalThis.location?.search ?? ''): number[] {
  const raw = new URLSearchParams(search).get('latency');
  if (!raw) return [];
  const values = raw
    .split(',')
    .map((part) => Number.parseInt(part.trim(), 10))
    .filter((value) => Number.isFinite(value) && value >= 0);
  return values;
}

let backend: ConversionBackend | null = null;

/** The live backend, created on first use. */
export function getBackend(): ConversionBackend {
  if (backend) return backend;
  const name = detectBackendName();
  if (name === 'tauri') {
    backend = createTauriBackend();
  } else {
    // The probe exists only in the browser build; the desktop app never grows a test surface.
    installTestHook('mock');
    backend = createMockBackend({ latency: parseLatency() });
  }
  return backend;
}

/** Replace the live backend. Tests and the app bootstrap only. */
export function setBackend(next: ConversionBackend | null): void {
  backend = next;
  if (next === null) resetTestHook();
}
