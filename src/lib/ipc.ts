/**
 * The only file in the frontend that is allowed to import `invoke`.
 *
 * Every call into the Rust core goes through a named, typed function declared here.
 * Nothing else imports `@tauri-apps/api`, which means the entire surface between the
 * WebView and the core is one short file that can be read in a single sitting. That is
 * the point: a boundary nobody can enumerate is a boundary nobody can audit.
 *
 * Rules for adding to this file.
 *
 * A command is a complete business operation, not a generic accessor. `unlockVault` is a
 * command; `getField` is not, because it turns the boundary into an open query interface
 * and invites calling it in a loop.
 *
 * The return types below describe what the Rust side sends. TypeScript types do not
 * exist at runtime, so they are a convenience for the reader and the compiler, never a
 * guarantee. The value is trusted because it comes from our own core, not because it is
 * annotated here. Anything that is not our own core is hostile input and gets validated.
 */

import { invoke } from '@tauri-apps/api/core';

/**
 * Which build produced the running binary.
 *
 * @public part of the boundary vocabulary, exported whether or not anything imports it
 * by name today.
 */
export type BuildProfile = 'debug' | 'release';

/** Name, version and build profile of the running application. */
export interface AppInfo {
  readonly name: string;
  readonly version: string;
  readonly profile: BuildProfile;
}

/**
 * Whether the encrypted database has been opened.
 *
 * Only one value exists until storage is implemented.
 *
 * @public part of the boundary vocabulary, exported whether or not anything imports it
 * by name today.
 */
export type DatabaseStatus = 'notInitialized';

/**
 * What the application will admit to about itself.
 *
 * Deliberately contains nothing that identifies the person or the machine: no paths, no
 * user name, no host name. A screenshot of the diagnostics screen is safe to share.
 */
export interface Diagnostics {
  readonly app: AppInfo;
  readonly os: string;
  readonly arch: string;
  readonly webviewVersion: string | null;
  readonly database: DatabaseStatus;
  readonly uptimeMs: number;
}

/** Reads the name, version and build profile of the running application. */
export async function fetchAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('app_info');
}

/** Reads a snapshot of the application state for the diagnostics screen. */
export async function fetchDiagnostics(): Promise<Diagnostics> {
  return invoke<Diagnostics>('diagnostics');
}
