import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { StoreApi } from 'zustand/vanilla';

/** Toasts are informational by default; failures get a quieter, tinted variant. */
export type ToastTone = 'info' | 'error';

export interface Toast {
  id: number;
  message: string;
  tone: ToastTone;
}

export interface ToastStore {
  toast: Toast | null;
  /** Show a transient message. Replaces any toast already on screen. */
  show: (message: string, tone?: ToastTone) => void;
  /** Dismiss immediately. */
  dismiss: () => void;
}

/** How long a toast stays up. Short on purpose - plan.md asks for "nothing more intrusive". */
export const TOAST_MS = 1800;

export function createToastStore(ttl: number = TOAST_MS): StoreApi<ToastStore> {
  let handle: ReturnType<typeof setTimeout> | null = null;
  let nextId = 0;

  return createStore<ToastStore>()((set) => ({
    toast: null,

    show(message, tone = 'info') {
      if (handle !== null) clearTimeout(handle);
      nextId += 1;
      const id = nextId;
      set({ toast: { id, message, tone } });
      handle = setTimeout(() => {
        handle = null;
        set((state) => (state.toast?.id === id ? { toast: null } : state));
      }, ttl);
    },

    dismiss() {
      if (handle !== null) clearTimeout(handle);
      handle = null;
      set({ toast: null });
    },
  }));
}

export const toastStore = createToastStore();

export function useToastStore<T>(selector: (state: ToastStore) => T): T {
  return useStore(toastStore, selector);
}
