import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';

// Settings Tauri expects: a fixed dev port (tauri.conf.json's devUrl),
// no screen clearing so Rust errors stay visible, and a build target the
// WebView2 (Chromium) runtime understands.
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: 'chrome110',
    outDir: 'dist',
    emptyOutDir: true,
  },
});
