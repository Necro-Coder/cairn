<script lang="ts">
  /**
   * What the header says about the vault: that it is open, how long it has left, and the
   * button that closes it now.
   *
   * The countdown is drawn from what the core reports rather than from a timer of its own.
   * The interface already receives the number every few seconds, so a second clock in here
   * would be a second opinion about when the vault closes, and the core's is the one that
   * decides.
   */
  import IconLockOpen from '../icons/IconLockOpen.svelte';
  import { session } from '../session.svelte';

  /**
   * How long before the automatic lock the countdown turns into a warning.
   *
   * Kept in step with `WARNING_BEFORE_LOCK_S` in `cairn-domain`, which is what documents
   * the choice. Nothing enforces the agreement and nothing needs to: a warning that
   * appeared slightly early would be a cosmetic mistake rather than a security one.
   */
  const WARNING_BEFORE_LOCK_S = 30;

  const remaining = $derived(session.status.idleRemainingS);
  const closingSoon = $derived(remaining !== null && remaining <= WARNING_BEFORE_LOCK_S);

  /** Minutes and seconds, the way a countdown is read. */
  function asClock(seconds: number): string {
    const minutes = Math.floor(seconds / 60);
    return `${String(minutes)}:${String(seconds % 60).padStart(2, '0')}`;
  }
</script>

<!-- Marked so a press here does not also hand the window to the window manager. -->
<div class="vault-state" data-no-drag>
  <span class="state" class:soon={closingSoon}>
    <IconLockOpen />
    {#if remaining === null}
      Abierta
    {:else}
      Abierta · se cierra en {asClock(remaining)}
    {/if}
  </span>

  <span class="hint"><kbd>Ctrl</kbd> <kbd>K</kbd></span>

  <button type="button" class="lock" onclick={() => void session.lock()}>
    Cerrar la caja fuerte
  </button>
</div>

<style>
  .vault-state {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    font-size: var(--text-sm);
    color: var(--colour-text-muted);
  }

  .state {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    white-space: nowrap;
  }

  /* Colour is never the only signal: the number beside it is already counting down, and
   * this only makes the last half minute easier to notice out of the corner of an eye. */
  .soon {
    color: var(--colour-warning);
  }

  .hint {
    display: flex;
    gap: var(--space-1);
  }

  .lock {
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: var(--colour-text);
    font-size: var(--text-sm);
    white-space: nowrap;
    transition: background-color var(--duration-fast) var(--easing);
  }

  .lock:hover {
    background-color: var(--colour-surface-sunken);
  }

  /*
   * Below the window minimum the header keeps the menu, the state and the palette
   * reminder, and drops the words on the button. The shortcut is still written above.
   */
  @media (max-width: 880px) {
    .hint {
      display: none;
    }
  }
</style>
