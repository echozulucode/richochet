import { invoke } from '@tauri-apps/api/core';

import type { ClipboardPayload, ConversionBackend, OutboundPayload, WireFormat } from '../types';

/**
 * The real backend: three commands from the frozen surface in `src-tauri/src/commands.rs`.
 *
 * Note `@tauri-apps/api/core` — `@tauri-apps/api/tauri` is the v1 path and does not exist here.
 */
export function createTauriBackend(): ConversionBackend {
  return {
    name: 'tauri',

    convert(input: string, from: WireFormat, to: WireFormat): Promise<string> {
      return invoke<string>('convert', { input, from, to });
    },

    readClipboard(): Promise<ClipboardPayload> {
      return invoke<ClipboardPayload>('read_clipboard');
    },

    writeClipboard(payload: OutboundPayload): Promise<void> {
      return invoke<void>('write_clipboard', { payload });
    },
  };
}
