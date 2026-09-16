/**
 * The only file in the frontend that is allowed to import `invoke`.
 *
 * Every call into the Rust core goes through a named, typed function declared here.
 * Nothing else imports `@tauri-apps/api`, which means the entire surface between the
 * WebView and the core is one short file that can be read in a single sitting. That is
 * the point: a boundary nobody can enumerate is a boundary nobody can audit.
 *
 * Nothing imports this file by path either. The rest of the frontend imports the `$ipc`
 * alias, and the build decides what is behind it: this module normally, and a stand-in
 * that invents its answers when the interface is being looked at in an ordinary browser.
 * Both are checked against `IpcSurface`, so neither can grow a function the other lacks.
 *
 * Rules for adding to this file.
 *
 * A command is a complete business operation, not a generic accessor. `unlockVault` is a
 * command; `getField` is not, because it turns the boundary into an open query interface
 * and invites calling it in a loop.
 *
 * The types describing what comes back live in `ipc.types.ts`, because they belong to the
 * boundary rather than to either side of it. TypeScript types do not exist at runtime, so
 * they are a convenience for the reader and the compiler, never a guarantee. The value is
 * trusted because it comes from our own core, not because it is annotated.
 */

import { invoke } from '@tauri-apps/api/core';

import type { AppInfo, Diagnostics, IpcSurface } from './ipc.types';

/** Reads the name, version and build profile of the running application. */
async function fetchAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('app_info');
}

/** Reads a snapshot of the application state for the diagnostics screen. */
async function fetchDiagnostics(): Promise<Diagnostics> {
  return invoke<Diagnostics>('diagnostics');
}

/**
 * The real boundary.
 *
 * Annotated with the shared type rather than merely happening to match it. Missing a
 * function fails to compile, and so does adding one that `IpcSurface` does not declare,
 * which is the half that keeps the stand-in honest: a new command cannot be added here
 * and quietly forgotten there.
 */
export const ipc: IpcSurface = {
  previewNotice: null,
  fetchAppInfo,
  fetchDiagnostics,
};
