<script lang="ts">
  import { ipc } from '$ipc';
  import type { LockReason, VaultError, VaultStatus } from '../lib/ipc.types';

  interface Props {
    /** How many attempts have failed, and how long attempts are still refused for. */
    status: VaultStatus;
    /** Why the vault closed, when it closed during this session. */
    lockReason: LockReason | null;
    /** Called with the status the core returned once the vault is open. */
    onunlocked: (status: VaultStatus) => void;
  }

  const { status, lockReason, onunlocked }: Props = $props();

  let password = $state('');
  let revealed = $state(false);
  let busy = $state(false);
  let problem = $state<string | null>(null);

  /**
   * Seconds still to wait, counted down here rather than asked for every second.
   *
   * Seeded from what the core reported and decremented locally. The core refuses regardless
   * of what this shows, so the worst a drifting countdown can do is be wrong about when to
   * offer the button again, and the next refusal corrects it.
   */
  let waiting = $derived(status.lockedOutForS);

  $effect(() => {
    if (waiting <= 0) {
      return;
    }
    const tick = setInterval(() => {
      waiting = Math.max(0, waiting - 1);
    }, 1000);
    return () => clearInterval(tick);
  });

  const lockExplanations: Record<LockReason, string> = {
    inactivity: 'La caja fuerte se ha cerrado sola por inactividad.',
    focusLost: 'La caja fuerte se ha cerrado al dejar de usar la ventana.',
    minimised: 'La caja fuerte se ha cerrado al minimizar la ventana.',
    requested: 'La caja fuerte está cerrada.',
  };

  const ready = $derived(!busy && password.length > 0 && waiting === 0);

  /**
   * Turns whatever the core refused with into a sentence.
   *
   * Every reason the vault did not open arrives as the same error, so there is exactly one
   * sentence for all of them. Saying more would mean the core had told this screen more, and
   * the difference between a wrong password and an edited header is the difference between
   * two things an attacker would very much like to be able to tell apart.
   */
  function explain(cause: unknown): string {
    const error = cause as Partial<VaultError> | null;

    switch (error?.kind) {
      case 'lockedOut':
        return 'Hay que esperar antes de volver a intentarlo.';
      case 'noVault':
        return 'Este equipo no tiene ninguna caja fuerte.';
      case 'passwordTooLong':
        return `La contraseña es demasiado larga: el máximo son ${String(error.max)} bytes.`;
      case 'derivationRefused':
        return 'Este equipo no ha podido ejecutar la derivación con los parámetros guardados.';
      default:
        return 'No se ha podido abrir la caja fuerte.';
    }
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!ready) {
      return;
    }

    busy = true;
    problem = null;
    // Copied out and the field emptied before the round trip. It never passes through a
    // store: from the field to the core and nowhere else.
    const typed = password;
    password = '';

    try {
      onunlocked(await ipc.unlockVault(typed));
    } catch (cause) {
      problem = explain(cause);
      const error = cause as Partial<VaultError> | null;
      if (error?.kind === 'lockedOut') {
        waiting = error.remainingS ?? waiting;
      }
    } finally {
      busy = false;
    }
  }
</script>

<section class="unlock">
  <header>
    <h1>Cairn</h1>
    <p class="lede">
      {lockReason === null ? 'La caja fuerte está cerrada.' : lockExplanations[lockReason]}
    </p>
  </header>

  {#if status.condition === 'restoredFromBackup'}
    <p class="notice" role="note">
      La cabecera no se pudo leer y se ha restaurado desde la copia que había al lado. La contraseña
      es la misma.
    </p>
  {/if}

  <form onsubmit={submit}>
    <div class="field">
      <label for="password">Contraseña maestra</label>
      <input
        id="password"
        type={revealed ? 'text' : 'password'}
        bind:value={password}
        autocomplete="current-password"
        spellcheck="false"
        disabled={busy || waiting > 0}
      />
    </div>

    <label class="toggle">
      <input type="checkbox" bind:checked={revealed} disabled={busy} />
      Mostrar lo que escribo
    </label>

    <button type="submit" disabled={!ready}>
      {busy ? 'Abriendo…' : 'Abrir'}
    </button>
  </form>

  <div role="status" aria-live="polite">
    {#if waiting > 0}
      <p class="problem">
        Demasiados intentos fallidos. Hay que esperar {waiting}
        {waiting === 1 ? 'segundo' : 'segundos'}.
      </p>
    {:else if problem !== null}
      <p class="problem">{problem}</p>
    {:else if busy}
      <p class="help">
        Derivando la clave. Tarda a propósito: es lo que hace cara cada prueba de quien intente
        adivinarla.
      </p>
    {/if}

    {#if status.failedAttempts > 0 && waiting === 0}
      <p class="help">
        {status.failedAttempts}
        {status.failedAttempts === 1 ? 'intento fallido' : 'intentos fallidos'} desde la última vez que
        se abrió.
      </p>
    {/if}
  </div>
</section>

<style>
  .unlock {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    max-width: 30rem;
  }

  header {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .lede {
    color: var(--colour-text-muted);
  }

  .notice {
    margin: 0;
    padding: var(--space-4);
    border: var(--border-width) solid var(--colour-warning);
    border-radius: var(--radius-lg);
    background-color: var(--colour-surface-raised);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  label {
    font-size: var(--text-sm);
    font-weight: 600;
  }

  input[type='password'],
  input[type='text'] {
    padding: var(--space-3);
    border: var(--border-width) solid var(--colour-border);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface);
    color: var(--colour-text);
    font-family: var(--font-mono);
  }

  .toggle {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    font-weight: 400;
  }

  .help {
    margin: 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .problem {
    margin: 0;
    color: var(--colour-negative);
    font-size: var(--text-sm);
  }

  button {
    align-self: flex-start;
    padding: var(--space-3) var(--space-5);
    border: var(--border-width) solid transparent;
    border-radius: var(--radius-md);
    background-color: var(--colour-accent);
    color: var(--colour-accent-contrast);
    font-weight: 600;
  }

  button:disabled {
    background-color: var(--colour-surface-raised);
    border-color: var(--colour-border);
    color: var(--colour-text-faint);
  }
</style>
