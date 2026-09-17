/**
 * A stand-in for the core, so that the interface can be opened in an ordinary browser.
 *
 * There is no Rust behind a browser tab. Without this module the screens would show nothing
 * but an error, which makes the interface impossible to look at anywhere except inside the
 * desktop window, and in particular impossible to look at on a phone before there is an
 * application to install on one.
 *
 * What this module is not, and what nothing here can be used to claim:
 *
 * - It validates nothing about the core. There is no Rust, no key derivation, no encryption
 *   and no database. A screen that works here can still be broken in the real application.
 * - **Nothing here is encrypted and nothing here is a secret.** The password is held as a
 *   plain string and compared with `===`. That is not a shortcut to be tidied up later; it is
 *   the reason this file may never reach a production bundle, and the gate that searches the
 *   built artefact for the marker below is what enforces it.
 * - The numbers it returns are invented. Reading a performance budget off a preview is
 *   reading a number somebody typed into this file.
 * - It is not a test double. Tests run against the real boundary or against Rust.
 *
 * Every value is fixed and deterministic. Random data, even seeded, hides intermittent
 * mistakes and means two screenshots of the same screen cannot be compared. The one thing
 * that does change is the clock, because a countdown that never counts cannot be looked at.
 *
 * This file is never in a production build. The `$ipc` alias only points here when Vite
 * runs in preview mode, so it is not behind a condition at runtime, it is absent from the
 * module graph entirely. A gate in the pipeline searches the built bundle for the marker
 * below and fails if it finds it, because a build flag on its own is not a defence: one
 * badly placed import undoes it without saying anything.
 */

import type {
  AppInfo,
  Diagnostics,
  InactivityChoice,
  IpcSurface,
  KdfParams,
  KdfReport,
  LockReason,
  PasswordStrength,
  VaultCondition,
  VaultError,
  VaultStatus,
} from '../ipc.types';

/**
 * The marker, shown to the person and searched for in the built bundle.
 *
 * One string with two jobs on purpose. Shown on screen it is unmistakable in a way that
 * a tasteful grey label is not, and a machine token in the middle of an interface is
 * exactly the kind of thing nobody mistakes for the real application. Searched for in
 * `dist/`, it is proof about the artefact rather than about the intention behind it.
 *
 * Because it is one string used in both places, the banner cannot say one thing while the
 * gate looks for another.
 */
const MARKER = 'CAIRN-PREVIEW-MOCK-DATA';

/** Obviously invented, and obviously not a version anybody released. */
const PREVIEW_APP_INFO: AppInfo = {
  name: 'Cairn (datos de mentira)',
  version: '0.0.0-ejemplo',
  profile: 'debug',
};

/** Obviously invented. No real system reports an architecture called this. */
const PREVIEW_DIAGNOSTICS: Diagnostics = {
  app: PREVIEW_APP_INFO,
  os: 'sistema de ejemplo',
  arch: 'arquitectura de ejemplo',
  webviewVersion: 'navegador de ejemplo',
  database: 'notInitialized',
  uptimeMs: 1234,
};

/** The fewest characters the real policy accepts. Kept in step with `cairn-domain`. */
const MIN_PASSWORD_CHARS = 12;

/** How long each inactivity choice lasts, in seconds. `never` has no entry. */
const INACTIVITY_SECONDS: Partial<Record<InactivityChoice, number>> = {
  one: 60,
  five: 300,
  fifteen: 900,
  thirty: 1800,
};

/** The longest the backoff grows to, matching the real schedule. */
const MAX_BACKOFF_S = 300;

/**
 * Everything the stand-in pretends to remember.
 *
 * None of it is stored anywhere. Reloading the page is a fresh machine with no vault, which
 * is the honest behaviour for something that keeps its state in a variable.
 */
interface PreviewVault {
  password: string;
  unlocked: boolean;
  failedAttempts: number;
  lockedUntilMs: number;
  lastActivityMs: number;
  inactivity: InactivityChoice;
  kdf: KdfReport;
  condition: VaultCondition;
}

let vault: PreviewVault | null = null;

/**
 * Whether the pretend window is maximised.
 *
 * A browser tab has no such state, so this exists only so that the button in the header
 * draws both of its glyphs when somebody presses it.
 */
let maximised = false;

/** The listeners the interface has registered for the lock event. */
const lockListeners = new Set<(reason: LockReason) => void>();

/** Tells every listener the vault has closed. */
function announceLock(reason: LockReason): void {
  for (const listener of lockListeners) {
    listener(reason);
  }
}

/**
 * Fails the way the real boundary fails, with the tagged object rather than a sentence.
 *
 * Not an `Error`, deliberately. Tauri rejects a command with whatever the core serialised,
 * which is this object, and a stand-in that rejected with something else would let a screen
 * be written against a shape the real boundary never produces.
 */
function reject(error: VaultError): Promise<never> {
  // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- the real boundary rejects with exactly this plain tagged object, and a stand-in that wrapped it in an Error would let a screen be written against a shape the core never produces
  return Promise.reject(error);
}

/** The same doubling schedule the core uses, capped in the same place. */
function backoffSeconds(failedAttempts: number): number {
  if (failedAttempts <= 0) {
    return 0;
  }
  return Math.min(2 ** (failedAttempts - 1), MAX_BACKOFF_S);
}

/** Seconds left of a wait that ends at `untilMs`, rounded up as the core rounds it. */
function remainingSeconds(untilMs: number): number {
  const left = untilMs - Date.now();
  return left <= 0 ? 0 : Math.min(Math.ceil(left / 1000), MAX_BACKOFF_S);
}

/** Closes the vault if its own clock says to, and says so, exactly as the watchdog does. */
function applyInactivity(): void {
  if (vault === null || !vault.unlocked) {
    return;
  }

  const seconds = INACTIVITY_SECONDS[vault.inactivity];
  if (seconds === undefined) {
    return;
  }

  if (Date.now() - vault.lastActivityMs >= seconds * 1000) {
    vault.unlocked = false;
    announceLock('inactivity');
  }
}

/** What the interface is told, assembled the way the core assembles it. */
function status(): VaultStatus {
  applyInactivity();

  if (vault === null) {
    return {
      exists: false,
      unlocked: false,
      condition: 'noVaultYet',
      kdf: null,
      failedAttempts: 0,
      lockedOutForS: 0,
      inactivity: 'five',
      idleRemainingS: null,
    };
  }

  const seconds = INACTIVITY_SECONDS[vault.inactivity];
  const idleRemainingS =
    vault.unlocked && seconds !== undefined
      ? Math.max(0, Math.ceil((vault.lastActivityMs + seconds * 1000 - Date.now()) / 1000))
      : null;

  return {
    exists: true,
    unlocked: vault.unlocked,
    condition: vault.condition,
    kdf: vault.kdf,
    failedAttempts: vault.failedAttempts,
    lockedOutForS: remainingSeconds(vault.lockedUntilMs),
    inactivity: vault.inactivity,
    idleRemainingS,
  };
}

/** The same two length rules the real policy applies, and no composition rules either. */
function validate(password: string): VaultError | null {
  const chars = [...password].length;
  if (chars < MIN_PASSWORD_CHARS) {
    return { kind: 'passwordTooShort', chars, min: MIN_PASSWORD_CHARS };
  }
  return null;
}

/** A report shaped like the one the core sends, from the parameters it was asked for. */
function report(params: KdfParams): KdfReport {
  return {
    memoryKib: params.memoryKib,
    passes: params.passes,
    lanes: params.lanes,
    writtenAtUs: Date.now() * 1000,
  };
}

/**
 * The stand-in boundary.
 *
 * Annotated with the same shared type as the real one, so the two cannot drift: a command
 * added on one side and not the other stops the build rather than waiting to be noticed.
 */
export const ipc: IpcSurface = {
  previewNotice: `Previsualización de la interfaz. Todos los datos son inventados, no hay núcleo detrás y aquí no se cifra nada. ${MARKER}`,

  fetchAppInfo: () => Promise.resolve(PREVIEW_APP_INFO),

  fetchDiagnostics: () => Promise.resolve(PREVIEW_DIAGNOSTICS),

  fetchVaultStatus: () => Promise.resolve(status()),

  createVault: (password, params) => {
    const problem = validate(password);
    if (problem !== null) {
      return reject(problem);
    }
    if (vault !== null) {
      return reject({ kind: 'alreadyExists' });
    }

    vault = {
      password,
      unlocked: true,
      failedAttempts: 0,
      lockedUntilMs: 0,
      lastActivityMs: Date.now(),
      inactivity: 'five',
      kdf: report(params),
      condition: 'readable',
    };

    return Promise.resolve(status());
  },

  unlockVault: (password) => {
    if (vault === null) {
      return reject({ kind: 'noVault' });
    }

    const waiting = remainingSeconds(vault.lockedUntilMs);
    if (waiting > 0) {
      return reject({ kind: 'lockedOut', remainingS: waiting });
    }

    // Compared with an ordinary equality, because there is nothing here to derive a key
    // from. See the warning at the top of this file.
    if (password !== vault.password) {
      vault.failedAttempts += 1;
      vault.lockedUntilMs = Date.now() + backoffSeconds(vault.failedAttempts) * 1000;
      return reject({ kind: 'notOpened' });
    }

    vault.unlocked = true;
    vault.failedAttempts = 0;
    vault.lockedUntilMs = 0;
    vault.lastActivityMs = Date.now();

    return Promise.resolve(status());
  },

  lockVault: () => {
    if (vault !== null && vault.unlocked) {
      vault.unlocked = false;
      announceLock('requested');
    }
    return Promise.resolve(status());
  },

  changeMasterPassword: (current, next) => {
    if (vault === null) {
      return reject({ kind: 'noVault' });
    }
    const problem = validate(next);
    if (problem !== null) {
      return reject(problem);
    }
    if (current !== vault.password) {
      return reject({ kind: 'notOpened' });
    }

    vault.password = next;
    vault.unlocked = true;
    vault.lastActivityMs = Date.now();

    return Promise.resolve(status());
  },

  changeKdfParams: (password, params) => {
    if (vault === null) {
      return reject({ kind: 'noVault' });
    }
    if (password !== vault.password) {
      return reject({ kind: 'notOpened' });
    }

    vault.kdf = report(params);
    vault.unlocked = true;
    vault.lastActivityMs = Date.now();

    return Promise.resolve(status());
  },

  sendHeartbeat: () => {
    if (vault === null || !vault.unlocked) {
      return reject({ kind: 'locked' });
    }

    vault.lastActivityMs = Date.now();

    return Promise.resolve(status());
  },

  setInactivity: (choice) => {
    if (vault !== null) {
      vault.inactivity = choice;
      vault.lastActivityMs = Date.now();
    }
    return Promise.resolve(status());
  },

  estimatePasswordStrength: (password) => {
    // A coarser guess than the core's, and deliberately so: this exists to make the bar
    // move while somebody looks at the screen, not to advise anyone about a password.
    const chars = [...password].length;
    const kinds = [/\p{Ll}/u, /\p{Lu}/u, /\p{Nd}/u, /[^\p{L}\p{Nd}]/u].filter((pattern) =>
      pattern.test(password),
    ).length;
    const score = chars + kinds * 4;

    let strength: PasswordStrength = 'weak';
    if (score >= 34) {
      strength = 'strong';
    } else if (score >= 26) {
      strength = 'good';
    } else if (score >= 18) {
      strength = 'fair';
    }

    return Promise.resolve(strength);
  },

  onVaultLocked: (handler) => {
    lockListeners.add(handler);
    return Promise.resolve(() => {
      lockListeners.delete(handler);
    });
  },

  /*
   * The four window controls.
   *
   * A browser tab is not a window this application owns: it cannot be dragged by its
   * content, it cannot be minimised, and closing it is not something a page may do
   * unasked. So these accept and do nothing, which is the honest behaviour — the buttons
   * are drawn and can be reached with the keyboard, and what they do belongs to a real
   * window.
   *
   * Only the maximise state is remembered, so that the button draws the right glyph and
   * somebody looking at the header can see both of them.
   */
  startWindowDrag: () => Promise.resolve(),

  minimizeWindow: () => Promise.resolve(),

  toggleMaximizeWindow: () => {
    maximised = !maximised;
    return Promise.resolve(maximised);
  },

  closeWindow: () => {
    // The vault half is real even here, and it is the half that matters: this is the one
    // place where a screen could be written against a close that left the vault open.
    if (vault !== null && vault.unlocked) {
      vault.unlocked = false;
      announceLock('requested');
    }
    return Promise.resolve();
  },
};
