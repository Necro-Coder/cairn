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
  BackupExportReport,
  BackupModule,
  BackupPasswordSource,
  BackupProgress,
  BackupStatus,
  BackupVerifyReport,
  ImportCommittedReport,
  ImportPreparedReport,
  PlaintextExportReport,
  Diagnostics,
  InactivityChoice,
  InstanceStatus,
  IpcSurface,
  KdfParams,
  KeysetPage,
  LockReason,
  PasswordStrength,
  SampleHabit,
  CompactionReport,
  SeedReport,
  VaultStatus,
  DayState,
  HabitDetail,
  HabitDraft,
  HabitFilter,
  HabitStats,
  HabitSummary,
  HabitUpdateOutcome,
  Heatmap,
  UpdateImpact,
} from './ipc.types';

/** The name the core sends the lock event under. It must match `window.rs`. */
const LOCKED_EVENT = 'session://locked';

/** The name the core sends progress under. It must match `commands/backup.rs`. */
const BACKUP_PROGRESS_EVENT = 'backup://progress';

/** Reads what this copy of the application is allowed to do with the data directory. */
async function fetchInstanceStatus(): Promise<InstanceStatus> {
  return invoke<InstanceStatus>('instance_status');
}

/** Reads the name, version and build profile of the running application. */
async function fetchAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('app_info');
}

/** Reads a snapshot of the application state for the diagnostics screen. */
async function fetchDiagnostics(): Promise<Diagnostics> {
  return invoke<Diagnostics>('diagnostics');
}

/** Writes one sample habit, to prove the whole path from the window to the file works. */
async function insertSampleHabit(): Promise<SampleHabit> {
  return invoke<SampleHabit>('diagnostics_insert_sample_habit');
}

/** Reads a page of sample habits, in clock order. */
async function listSampleHabits(page: KeysetPage): Promise<readonly SampleHabit[]> {
  return invoke<SampleHabit[]>('diagnostics_list_sample_habits', { page });
}

/** Marks a sample habit as deleted and empties its encrypted column. */
async function deleteSampleHabit(id: string): Promise<SampleHabit> {
  return invoke<SampleHabit>('diagnostics_delete_sample_habit', { id });
}

/** Writes a number of rows into every table that has a generator, for measuring. */
async function seedData(rowsPerTable: number): Promise<SeedReport> {
  return invoke<SeedReport>('diagnostics_seed_data', { rowsPerTable });
}

/** Removes the tombstones older than the retention window the core keeps. */
async function compactTombstones(): Promise<CompactionReport> {
  return invoke<CompactionReport>('diagnostics_compact_tombstones');
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

/** Writes everything in the vault to one encrypted file, and reads it back before saying so. */
async function exportBackup(
  password: string,
  source: BackupPasswordSource,
): Promise<BackupExportReport> {
  return invoke<BackupExportReport>('backup_export', { password, source });
}

/** Reads a backup end to end and reports what is in it, writing nothing. */
async function verifyBackup(password: string): Promise<BackupVerifyReport> {
  return invoke<BackupVerifyReport>('backup_verify', { password });
}

/** Says how long it has been since the last backup, and whether to mention it. */
async function backupStatus(): Promise<BackupStatus> {
  return invoke<BackupStatus>('backup_status');
}

/** Reads a backup into a database of its own beside the live one, touching nothing. */
async function beginImport(password: string): Promise<ImportPreparedReport> {
  return invoke<ImportPreparedReport>('backup_import_begin', { password });
}

/** Replaces the vault with the one that was prepared, after copying the old one aside. */
async function commitImport(token: string): Promise<ImportCommittedReport> {
  return invoke<ImportCommittedReport>('backup_import_commit', { token });
}

/** Throws away a prepared import and the staging database it wrote. */
async function cancelImport(token: string): Promise<void> {
  return invoke<void>('backup_import_cancel', { token });
}

/** Writes one module out as a file anybody can read, after the master password is checked. */
async function exportPlaintext(
  module: BackupModule,
  password: string,
): Promise<PlaintextExportReport> {
  return invoke<PlaintextExportReport>('export_plaintext', { module, password });
}

/** Listens for how far along a running export or verification is. */
async function onBackupProgress(handler: (progress: BackupProgress) => void): Promise<() => void> {
  return listen<BackupProgress>(BACKUP_PROGRESS_EVENT, (event) => {
    handler(event.payload);
  });
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

/* -----------------------------------------------------------------------------------------
 * Habits.
 *
 * Eleven commands. The name in `invoke` is the core's, in `snake_case` and prefixed by the
 * module, because the core has one flat namespace and `list` in it would be a command about
 * nothing in particular. The name of the function is this side's. The two are deliberately
 * different and this file is the only place they meet.
 *
 * The argument names in each object are the parameter names of the Rust function, converted
 * the way Tauri converts them. Getting one wrong is not a type error anywhere: it is an
 * argument the command never receives.
 * -------------------------------------------------------------------------------------- */

/** Reads every habit of one kind, with today's square and the run so far. */
async function listHabits(filter: HabitFilter): Promise<readonly HabitSummary[]> {
  return invoke<HabitSummary[]>('habits_list', { filter });
}

/** Reads one habit in full, note included. */
async function getHabit(id: string): Promise<HabitDetail> {
  return invoke<HabitDetail>('habits_get', { id });
}

/** Creates a habit from a draft. */
async function createHabit(draft: HabitDraft): Promise<HabitDetail> {
  return invoke<HabitDetail>('habits_create', { draft });
}

/** Saves a draft over an existing habit. */
async function updateHabit(id: string, draft: HabitDraft): Promise<HabitUpdateOutcome> {
  return invoke<HabitUpdateOutcome>('habits_update', { id, draft });
}

/** Works out what saving that draft would do, without saving it. */
async function previewHabitUpdate(id: string, draft: HabitDraft): Promise<UpdateImpact> {
  return invoke<UpdateImpact>('habits_update_preview', { id, draft });
}

/** Puts a habit away, or takes it back out. */
async function archiveHabit(id: string, archived: boolean): Promise<HabitSummary> {
  return invoke<HabitSummary>('habits_archive', { id, archived });
}

/** Removes a habit and every day ever marked on it. */
async function deleteHabit(id: string): Promise<void> {
  return invoke<void>('habits_delete', { id });
}

/** Sets the person's whole order at once. */
async function reorderHabits(ids: readonly string[]): Promise<void> {
  return invoke<void>('habits_reorder', { ids });
}

/** Marks or unmarks one day, and answers what that square now says. */
async function toggleHabitDay(id: string, day: number, amount: number | null): Promise<DayState> {
  return invoke<DayState>('habits_toggle_day', { id, day, amount });
}

/** Reads one whole year of one habit. */
async function habitHeatmap(id: string, year: number): Promise<Heatmap> {
  return invoke<Heatmap>('habits_heatmap', { id, year });
}

/** Reads everything the detail screen shows in numbers. */
async function habitStats(id: string): Promise<HabitStats> {
  return invoke<HabitStats>('habits_stats', { id });
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
  fetchInstanceStatus,
  fetchAppInfo,
  fetchDiagnostics,
  insertSampleHabit,
  listSampleHabits,
  deleteSampleHabit,
  seedData,
  compactTombstones,
  fetchVaultStatus,
  createVault,
  unlockVault,
  lockVault,
  changeMasterPassword,
  changeKdfParams,
  sendHeartbeat,
  setInactivity,
  estimatePasswordStrength,
  exportBackup,
  verifyBackup,
  backupStatus,
  beginImport,
  commitImport,
  cancelImport,
  exportPlaintext,
  onBackupProgress,
  onVaultLocked,
  startWindowDrag,
  minimizeWindow,
  toggleMaximizeWindow,
  closeWindow,
  listHabits,
  getHabit,
  createHabit,
  updateHabit,
  previewHabitUpdate,
  archiveHabit,
  deleteHabit,
  reorderHabits,
  toggleHabitDay,
  habitHeatmap,
  habitStats,
};
