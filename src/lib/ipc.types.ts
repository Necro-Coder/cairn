/**
 * The vocabulary of the boundary between the interface and the core, and the shape any
 * implementation of that boundary has to have.
 *
 * This file imports nothing. It describes the frontier without being on either side of
 * it, which is what lets two different modules be checked against the same contract: the
 * one that calls the Rust core, and the one that stands in for it while the interface is
 * being looked at in an ordinary browser.
 *
 * The point of `IpcSurface` is that the two cannot drift apart. Add a function to one and
 * not the other and the build stops, rather than the stand-in quietly falling behind until
 * somebody designs a screen against a boundary that no longer matches the real one.
 *
 * TypeScript types do not exist at runtime, so nothing here is a guarantee about a value.
 * What arrives from the core is trusted because it comes from our own core, not because it
 * is annotated. Anything that is not our own core is hostile input and gets validated.
 */

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

/**
 * Everything the interface may ask of whatever is on the other side of the boundary.
 *
 * Every implementation is checked against this, so a function that exists on one side and
 * not the other is a compile error rather than a surprise at runtime. That is the whole
 * reason the type exists; a written convention would survive exactly until the first time
 * somebody was in a hurry.
 */
export interface IpcSurface {
  /**
   * What to put on screen to say that none of this is real, or `null` when it is.
   *
   * It belongs to the implementation rather than to the interface because the module that
   * makes the data up is the one that knows it did. The real boundary answers `null`, so
   * the warning text does not exist anywhere in a production build: it is not hidden by a
   * condition, it is simply not there.
   */
  readonly previewNotice: string | null;

  /** Reads the name, version and build profile of the running application. */
  readonly fetchAppInfo: () => Promise<AppInfo>;

  /** Reads a snapshot of the application state for the diagnostics screen. */
  readonly fetchDiagnostics: () => Promise<Diagnostics>;

  /** Reads everything the interface needs to decide what to draw about the vault. */
  readonly fetchVaultStatus: () => Promise<VaultStatus>;

  /** Creates the vault and opens it. Rejects with a {@link VaultError}. */
  readonly createVault: (password: string, params: KdfParams) => Promise<VaultStatus>;

  /** Opens the vault. Rejects with a {@link VaultError}. */
  readonly unlockVault: (password: string) => Promise<VaultStatus>;

  /** Closes the vault, clearing every key in the core. */
  readonly lockVault: () => Promise<VaultStatus>;

  /** Changes the master password, keeping every stored byte as it is. */
  readonly changeMasterPassword: (current: string, next: string) => Promise<VaultStatus>;

  /** Changes the derivation parameters, keeping the password and every stored byte. */
  readonly changeKdfParams: (password: string, params: KdfParams) => Promise<VaultStatus>;

  /** Reports keyboard or mouse activity inside the window. */
  readonly sendHeartbeat: () => Promise<VaultStatus>;

  /** Changes how long the vault may sit idle before it closes itself. */
  readonly setInactivity: (choice: InactivityChoice) => Promise<VaultStatus>;

  /**
   * Estimates how strong a password looks.
   *
   * Asked of the core rather than computed here, because the alternative puts several
   * megabytes of word list into the bundle and evaluates the master password in JavaScript.
   */
  readonly estimatePasswordStrength: (password: string) => Promise<PasswordStrength>;

  /**
   * Listens for the vault closing, and hands back the way to stop listening.
   *
   * The only event that crosses the boundary. Everything else the interface wants it asks
   * for, because an event carrying state is an event that can be missed.
   */
  readonly onVaultLocked: (handler: (reason: LockReason) => void) => Promise<() => void>;

  /**
   * Hands the window to the window manager for a drag. Rejects with a {@link WindowError}.
   *
   * The window has no system decoration, so the header is the title bar and these four are
   * what a title bar does. They are commands of our own rather than entries in the
   * capability list: `core:window:*` would add four core APIs to what script injected into
   * the WebView could call, and a custom command needs no entry at all.
   */
  readonly startWindowDrag: () => Promise<void>;

  /** Minimises the window. Rejects with a {@link WindowError}. */
  readonly minimizeWindow: () => Promise<void>;

  /**
   * Maximises the window or restores it, answering which it now is.
   *
   * Answering rather than leaving the interface to ask again means the button redraws
   * itself from the result of the press rather than from a second round trip.
   */
  readonly toggleMaximizeWindow: () => Promise<boolean>;

  /**
   * Closes the vault and then the window, in that order.
   *
   * The order is the core's, not this side's, and it is not negotiable: closing the window
   * first would tear down the WebView with a key still live in the process.
   */
  readonly closeWindow: () => Promise<void>;
}

/**
 * Why a window control did not do what it was asked.
 *
 * One reason, because none of the four takes a parameter: either the window is there and
 * the window manager agreed, or it is not.
 */
export type WindowError = { readonly kind: 'unavailable' };

/**
 * What reading the vault header at startup found.
 *
 * `unreadable` is the one that changes what the interface may offer. A header that is there
 * and cannot be parsed is still a vault, so the screen that appears has to be the one about
 * restoring a copy and never the one about creating a vault.
 */
export type VaultCondition = 'noVaultYet' | 'readable' | 'restoredFromBackup' | 'unreadable';

/**
 * How long the vault may sit idle before it closes itself.
 *
 * A closed set on both sides of the boundary, so a period nobody designed for cannot be sent
 * across and become the lock policy.
 */
export type InactivityChoice = 'one' | 'five' | 'fifteen' | 'thirty' | 'never';

/**
 * Why the vault closed.
 *
 * Carried by the only event the core sends, so that the screen which appears can say what
 * happened instead of arriving for no visible reason.
 */
export type LockReason = 'inactivity' | 'focusLost' | 'minimised' | 'requested';

/** How strong a password looks. An estimate, never a measurement, and it never blocks. */
export type PasswordStrength = 'weak' | 'fair' | 'good' | 'strong';

/** The Argon2id parameters currently in force. None of it is secret. */
export interface KdfReport {
  readonly memoryKib: number;
  readonly passes: number;
  readonly lanes: number;
  readonly writtenAtUs: number;
}

/** Everything the interface needs to decide what to draw. */
export interface VaultStatus {
  readonly exists: boolean;
  readonly unlocked: boolean;
  readonly condition: VaultCondition;
  readonly kdf: KdfReport | null;
  readonly failedAttempts: number;
  readonly lockedOutForS: number;
  readonly inactivity: InactivityChoice;
  /** Seconds left before the vault closes itself, or `null` when it is closed or never does. */
  readonly idleRemainingS: number | null;
}

/**
 * Why an operation on the vault did not happen.
 *
 * Every reason the vault failed to open is `notOpened`, on purpose: telling a wrong password
 * apart from an edited header would say which half of the problem to work on. The tag is what
 * the interface matches on, and the numbers beside it are what it puts in a sentence.
 */
export type VaultError =
  | { readonly kind: 'notOpened' }
  | { readonly kind: 'lockedOut'; readonly remainingS: number }
  | { readonly kind: 'noVault' }
  | { readonly kind: 'alreadyExists' }
  | { readonly kind: 'locked' }
  | { readonly kind: 'passwordTooShort'; readonly chars: number; readonly min: number }
  | { readonly kind: 'passwordTooLong'; readonly bytes: number; readonly max: number }
  | { readonly kind: 'passwordRejected' }
  | {
      readonly kind: 'paramOutOfRange';
      readonly field: string;
      readonly value: number;
      readonly min: number;
      readonly max: number;
    }
  | { readonly kind: 'derivationRefused' }
  | { readonly kind: 'storage' };

/** The Argon2id parameters an operation is asked to use. */
export interface KdfParams {
  readonly memoryKib: number;
  readonly passes: number;
  readonly lanes: number;
}
