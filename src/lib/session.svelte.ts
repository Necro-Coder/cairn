/**
 * What the interface knows about the vault, and what it throws away when the vault closes.
 *
 * Two jobs, and the second one is the reason this file exists.
 *
 * It holds the status the core reports, so that every screen is drawing the same answer
 * rather than each asking for its own. And it listens for the one event the core sends, so
 * that when the vault closes for a reason nobody in here decided, everything derived from an
 * open vault is discarded in one place instead of in however many screens happened to be
 * showing something.
 *
 * What gets discarded is everything: the tabs, the panel and what has been run from the
 * palette. It is held in `workspace.svelte.ts` as one object precisely so that discarding it
 * is one assignment, and this file is what decides when that happens.
 *
 * Nothing is kept across a lock, not even which screen was open. An earlier version of this
 * file kept that, on the argument that a module name is not worth protecting, and it is not.
 * What is worth protecting is the promise the lock makes: somebody who locks the window with
 * six tabs open and comes back to it finds the application as it is on a fresh unlock, with
 * no trace in the strip and none in the palette of what they were doing.
 *
 * No key and nothing decrypted is ever here, because no key and nothing decrypted ever
 * crosses the boundary. The password is not here either: it lives in the field somebody typed
 * it into, is passed straight to the core, and the field is cleared.
 */

import { ipc } from '$ipc';
import type { InactivityChoice, LockReason, VaultStatus } from './ipc.types';
import { workspace } from './shell/workspace.svelte';

/**
 * How often the interface reports that somebody is using it.
 *
 * Five seconds. The thing being measured is minutes, so anything finer is round trips nobody
 * benefits from, and anything coarser makes the countdown the screen draws visibly wrong.
 */
const HEARTBEAT_INTERVAL_MS = 5_000;

/** The status of a machine nobody has asked yet. */
const UNKNOWN: VaultStatus = {
  exists: false,
  unlocked: false,
  condition: 'noVaultYet',
  kdf: null,
  failedAttempts: 0,
  lockedOutForS: 0,
  inactivity: 'five',
  idleRemainingS: null,
};

/** The session, as one piece of state the whole interface reads. */
class Session {
  /** The last answer the core gave. */
  status = $state<VaultStatus>(UNKNOWN);

  /** Why the vault closed, while there is still something to say about it. */
  lockReason = $state<LockReason | null>(null);

  /** Stops the heartbeat and the event listener. */
  #stop: (() => void) | null = null;

  /** Whether there has been keyboard or mouse activity since the last beat was sent. */
  #touched = false;

  /**
   * Records that somebody used the window.
   *
   * Called from the key and pointer handlers, and it does nothing but set a flag. The round
   * trip happens on the timer, which is what keeps a burst of typing from becoming a burst of
   * crossings: the core only needs to know that somebody was here in the last five seconds,
   * not how many times.
   */
  noteActivity(): void {
    this.#touched = true;
  }

  /** Reads the status once, without touching anything else. */
  async refresh(): Promise<VaultStatus> {
    const next = await ipc.fetchVaultStatus();
    this.#apply(next);
    return next;
  }

  /**
   * Records a status the caller already has, so an unlock does not cost a second round trip.
   */
  adopt(next: VaultStatus): void {
    this.#apply(next);
  }

  /** Changes how long the vault may sit idle. */
  async setInactivity(choice: InactivityChoice): Promise<void> {
    this.#apply(await ipc.setInactivity(choice));
  }

  /** Closes the vault. */
  async lock(): Promise<void> {
    this.#apply(await ipc.lockVault());
  }

  /**
   * Starts listening for the lock event and reporting activity, and hands back the way to
   * stop.
   *
   * The heartbeat is sent on a timer rather than on every key press, and only while the vault
   * is open. A round trip per keystroke would be several hundred crossings a minute to
   * communicate a fact that changes on the scale of minutes.
   */
  async start(): Promise<() => void> {
    // The timer first, and deliberately. Subscribing crosses to the core and the core is
    // entitled to refuse: if that rejection took this function with it, the timer below
    // would never be installed and a vault that closed itself would go on looking open
    // until something else asked. The event is how the screen changes at once; the timer is
    // what guarantees it changes at all.
    const beat = setInterval(() => {
      if (!this.status.unlocked) {
        return;
      }
      if (!this.#touched) {
        // Nobody has touched the window since the last beat, so there is nothing to report.
        // Sending one anyway would hold the vault open for an empty room, which is precisely
        // what the inactivity timer exists to prevent. The status is still refreshed, because
        // the countdown on screen has to keep counting.
        void this.refresh().catch(() => undefined);
        return;
      }
      this.#touched = false;

      // A failure means the core says the vault is closed while this thought it was open, so
      // the answer is to find out rather than to retry.
      void ipc
        .sendHeartbeat()
        .then((next) => this.#apply(next))
        .catch(() => this.refresh().catch(() => undefined));
    }, HEARTBEAT_INTERVAL_MS);

    // Recorded before the subscription is attempted, so that a refusal leaves something
    // that can still stop the timer instead of leaking it.
    this.#stop = () => {
      clearInterval(beat);
    };

    let unlisten: () => void;
    try {
      unlisten = await ipc.onVaultLocked((reason) => {
        this.#discard(reason);
      });
    } catch (cause) {
      // Not swallowed. The timer above keeps the interface honest within one beat, which is
      // a degradation rather than a failure, but something refused a subscription this
      // application needs and that is not a thing to carry on quietly from.
      throw new Error('the interface could not subscribe to the lock event', { cause });
    }

    this.#stop = () => {
      clearInterval(beat);
      unlisten();
    };

    return this.#stop;
  }

  /** Stops listening. Safe to call when it never started. */
  stop(): void {
    this.#stop?.();
    this.#stop = null;
  }

  /** Records a status, discarding the workspace if the core says the vault is closed. */
  #apply(next: VaultStatus): void {
    const wasOpen = this.status.unlocked || workspace.state !== null;
    this.status = next;

    if (next.unlocked) {
      this.lockReason = null;
      workspace.follow(true);
      return;
    }

    if (wasOpen) {
      // Closed between two answers without an event arriving, which happens when a command
      // is what closed it. The state goes the same way it would have gone on the event.
      this.#discard(this.lockReason ?? 'requested');
    }
  }

  /**
   * Throws away everything that came from an open vault.
   *
   * One call, and it takes the tabs, the panel and the palette history with it. Which screen
   * was open goes too: the strip is what somebody was doing, and a lock that left it behind
   * would be a lock that kept a record of the afternoon on an unattended window.
   */
  #discard(reason: LockReason): void {
    workspace.follow(false);
    this.lockReason = reason;
    this.status = { ...this.status, unlocked: false, idleRemainingS: null };
  }
}

/** The one session the whole interface shares. */
export const session = new Session();
