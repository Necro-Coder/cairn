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
 *
 * Nothing that crosses this boundary is a key or anything decrypted with one. Passwords go
 * one way only, and what comes back is booleans, counts and enumerations.
 */

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import type {
  AppInfo,
  Diagnostics,
  InactivityChoice,
  IpcSurface,
  KdfParams,
  LockReason,
  PasswordStrength,
  VaultStatus,
} from './ipc.types';

/** The name the core sends the one event under. It must match `window.rs`. */
const LOCKED_EVENT = 'session://locked';

/** Reads the name, version and build profile of the running application. */
async function fetchAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('app_info');
}

/** Reads a snapshot of the application state for the diagnostics screen. */
async function fetchDiagnostics(): Promise<Diagnostics> {
  return invoke<Diagnostics>('diagnostics');
}

/** Reads everything the interface needs to decide what to draw about the vault. */
async function fetchVaultStatus(): Promise<VaultStatus> {
  return invoke<VaultStatus>('vault_status');
}

/** Creates the vault and opens it. */
async function createVault(password: string, params: KdfParams): Promise<VaultStatus> {
  return invoke<VaultStatus>('vault_create', {
    password,
    memoryKib: params.memoryKib,
    passes: params.passes,
    lanes: params.lanes,
  });
}

/** Opens the vault. */
async function unlockVault(password: string): Promise<VaultStatus> {
  return invoke<VaultStatus>('vault_unlock', { password });
}

/** Closes the vault, clearing every key in the core. */
async function lockVault(): Promise<VaultStatus> {
  return invoke<VaultStatus>('vault_lock');
}

/** Changes the master password, keeping every stored byte as it is. */
async function changeMasterPassword(current: string, next: string): Promise<VaultStatus> {
  return invoke<VaultStatus>('vault_change_password', { current, new: next });
}

/** Changes the derivation parameters, keeping the password and every stored byte. */
async function changeKdfParams(password: string, params: KdfParams): Promise<VaultStatus> {
  return invoke<VaultStatus>('vault_change_kdf_params', {
    password,
    memoryKib: params.memoryKib,
    passes: params.passes,
    lanes: params.lanes,
  });
}

/** Reports keyboard or mouse activity inside the window. */
async function sendHeartbeat(): Promise<VaultStatus> {
  return invoke<VaultStatus>('session_heartbeat');
}

/** Changes how long the vault may sit idle before it closes itself. */
async function setInactivity(choice: InactivityChoice): Promise<VaultStatus> {
  return invoke<VaultStatus>('session_set_inactivity', { inactivity: choice });
}

/** Estimates how strong a password looks. */
async function estimatePasswordStrength(password: string): Promise<PasswordStrength> {
  return invoke<PasswordStrength>('password_strength', { password });
}

/** Listens for the vault closing, and hands back the way to stop listening. */
async function onVaultLocked(handler: (reason: LockReason) => void): Promise<() => void> {
  return listen<{ reason: LockReason }>(LOCKED_EVENT, (event) => {
    handler(event.payload.reason);
  });
}

/** Hands the window to the window manager for a drag. */
async function startWindowDrag(): Promise<void> {
  return invoke<void>('start_window_drag');
}

/** Minimises the window. */
async function minimizeWindow(): Promise<void> {
  return invoke<void>('minimize_window');
}

/** Maximises the window or restores it, answering which it now is. */
async function toggleMaximizeWindow(): Promise<boolean> {
  return invoke<boolean>('toggle_maximize_window');
}

/** Closes the vault and then the window, in that order. */
async function closeWindow(): Promise<void> {
  return invoke<void>('close_window');
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
  fetchVaultStatus,
  createVault,
  unlockVault,
  lockVault,
  changeMasterPassword,
  changeKdfParams,
  sendHeartbeat,
  setInactivity,
  estimatePasswordStrength,
  onVaultLocked,
  startWindowDrag,
  minimizeWindow,
  toggleMaximizeWindow,
  closeWindow,
};
