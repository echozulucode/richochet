import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { StoreApi } from 'zustand/vanilla';

/**
 * Theme preference. 'system' is the default and the only value that leaves the document alone -
 * the palette then follows prefers-color-scheme entirely in CSS.
 */
export type ThemePreference = 'system' | 'light' | 'dark';

const STORAGE_KEY = 'richochet.theme';

export interface ThemeStore {
  preference: ThemePreference;
  setPreference: (preference: ThemePreference) => void;
}

function readStored(): ThemePreference {
  try {
    const value = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (value === 'light' || value === 'dark' || value === 'system') return value;
  } catch {
    // Private mode or a locked-down WebView; the default is fine.
  }
  return 'system';
}

/** Stamp (or clear) data-theme on <html>. CSS does the rest. */
export function applyTheme(preference: ThemePreference): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  if (preference === 'system') {
    root.removeAttribute('data-theme');
  } else {
    root.setAttribute('data-theme', preference);
  }
}

export function createThemeStore(): StoreApi<ThemeStore> {
  return createStore<ThemeStore>()((set) => ({
    preference: readStored(),
    setPreference(preference) {
      applyTheme(preference);
      try {
        globalThis.localStorage?.setItem(STORAGE_KEY, preference);
      } catch {
        // Not being able to remember the choice is not worth an error.
      }
      set({ preference });
    },
  }));
}

export const themeStore = createThemeStore();

export function useThemeStore<T>(selector: (state: ThemeStore) => T): T {
  return useStore(themeStore, selector);
}
