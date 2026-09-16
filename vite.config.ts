import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

// Tauri drives this dev server, so the port is fixed and failing loudly is better than
// silently moving to another port that the desktop window is not pointing at.
const DEV_SERVER_PORT = 1420;

/** The mode that swaps the core for a module that makes its answers up. */
const PREVIEW_MODE = 'preview';

/**
 * Where the boundary lives, written the way Vite reads a path: from the project root
 * rather than from the machine.
 *
 * Deliberately not built with Node's path helpers. Doing that would mean adding Node's
 * type definitions to a project that has none, for the sake of two string constants, and
 * a leading slash already means "from the root" to every resolver in this build.
 */
const REAL_BOUNDARY = '/src/lib/ipc.ts';
const PREVIEW_BOUNDARY = '/src/lib/preview/ipc.ts';

export default defineConfig(({ mode }) => {
  const preview = mode === PREVIEW_MODE;

  return {
    plugins: [svelte()],
    // Tauri prints its own diagnostics to this terminal; clearing it would hide them.
    clearScreen: false,
    resolve: {
      alias: {
        // The whole frontend imports `$ipc` and never a path, so which implementation is
        // behind the boundary is decided here, once, at build time.
        //
        // An alias rather than a flag checked while the application runs. A flag would
        // leave the module that invents data present in every release, one mistaken
        // condition away from being reached. This way it is not in the graph at all: there
        // is nothing to reach.
        $ipc: preview ? PREVIEW_BOUNDARY : REAL_BOUNDARY,
      },
    },
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
      // A preview build never goes near `dist/`. Sharing the directory would mean one
      // absent-minded command could leave a bundle full of invented data sitting where
      // the packaging step looks for the real one.
      outDir: preview ? 'dist-preview' : 'dist',
      // Matches the WebView engines actually shipped: WebView2 on Windows and WKWebView on
      // iOS. Targeting anything older would mean shipping polyfills nobody executes.
      target: ['chrome110', 'safari16'],
      // Source maps would ship a readable copy of the frontend inside the binary.
      sourcemap: false,
      // Oxc is the minifier this version of Vite ships with. Naming esbuild instead would
      // pull in a second toolchain as an extra dependency for no gain.
      minify: 'oxc',
    },
  };
});
