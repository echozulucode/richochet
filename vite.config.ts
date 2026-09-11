import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';

// Tailwind 4 is CSS-first: the plugin below plus `@import "tailwindcss";` in src/styles.css.
// There is deliberately no tailwind.config.js and no PostCSS step.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Tauri reads the dev server from src-tauri/tauri.conf.json (devUrl http://localhost:5173),
  // so the port is not negotiable.
  server: {
    port: 5173,
    strictPort: true,
  },
  preview: {
    port: 5173,
    strictPort: true,
  },
  clearScreen: false,
  build: {
    target: 'chrome110',
    sourcemap: true,
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test-support/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    restoreMocks: true,
  },
});
