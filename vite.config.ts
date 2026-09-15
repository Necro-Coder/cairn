import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Tauri drives this dev server, so the port is fixed and failing loudly is better than
// silently moving to another port that the desktop window is not pointing at.
const DEV_SERVER_PORT = 1420;

export default defineConfig({
  plugins: [svelte()],
  // Tauri prints its own diagnostics to this terminal; clearing it would hide them.
  clearScreen: false,
  server: {
    port: DEV_SERVER_PORT,
    strictPort: true,
    host: '127.0.0.1',
    watch: {
      // The Rust side has its own rebuild loop. Watching it here would restart Vite on
      // every cargo write for no benefit.
      ignored: ['**/src-tauri/**', '**/crates/**', '**/target/**'],
    },
  },
  build: {
    // Matches the WebView engines actually shipped: WebView2 on Windows and WKWebView on
    // iOS. Targeting anything older would mean shipping polyfills nobody executes.
    target: ['chrome110', 'safari16'],
    // Source maps would ship a readable copy of the frontend inside the binary.
    sourcemap: false,
    // Oxc is the minifier this version of Vite ships with. Naming esbuild instead would
    // pull in a second toolchain as an extra dependency for no gain.
    minify: 'oxc',
  },
});
