<script lang="ts">
  /**
   * Creating the vault: the first screen anybody sees, and the only one where a mistake
   * cannot be undone.
   *
   * It is one of the two screens held to AAA contrast rather than AA, so the secondary text
   * uses the stronger muted colour. It is also one of the three screens with no data on them,
   * which is why it is allowed a reduced composition — two shapes, in their own region, with
   * nothing on top of them.
   *
   * The warning comes before the fields and not after. A warning underneath a form is a
   * warning read once the decision is made, and this one is about losing everything.
   */
  import { ipc } from '$ipc';
  import Marks from '../lib/shell/Marks.svelte';
  import type { KdfParams, PasswordStrength, VaultError, VaultStatus } from '../lib/ipc.types';

  interface Props {
    /** Called with the status the core returned once the vault exists and is open. */
    oncreated: (status: VaultStatus) => void;
  }

  const { oncreated }: Props = $props();

  /**
   * What a new vault is created with.
   *
   * Written here rather than chosen by the person, because somebody creating a vault has no
   * way to judge these and the screen that does let them change them, in the diagnostics
   * panel, exists for a machine that has already been measured. They can be raised later
   * without losing a single stored byte, which is the whole reason the key hierarchy is
   * shaped the way it is.
   */
  const INITIAL_PARAMS: KdfParams = { memoryKib: 65_536, passes: 3, lanes: 1 };

  /** Kept in step with the policy in the core, which is what actually refuses. */
  const MIN_CHARS = 12;

  let password = $state('');
  let repeated = $state('');
  let revealed = $state(false);
  let acknowledged = $state(false);
  let strength = $state<PasswordStrength | null>(null);
  let busy = $state(false);
  let problem = $state<string | null>(null);

  const chars = $derived([...password].length);
  const tooShort = $derived(chars > 0 && chars < MIN_CHARS);
  const mismatched = $derived(repeated.length > 0 && repeated !== password);
  const ready = $derived(
    !busy && chars >= MIN_CHARS && repeated === password && acknowledged && !mismatched,
  );

  const strengthLabels: Record<PasswordStrength, string> = {
    weak: 'débil',
    fair: 'aceptable',
    good: 'buena',
    strong: 'muy buena',
  };

  /**
   * Asks the core how strong the password looks.
   *
   * The password goes across for this, which is the same journey it makes to create the
   * vault a moment later. The alternative is several megabytes of word list in the bundle
   * and the master password evaluated in JavaScript, which is a worse trade.
   */
  async function estimate(): Promise<void> {
    if (password.length === 0) {
      strength = null;
      return;
    }
    try {
      strength = await ipc.estimatePasswordStrength(password);
    } catch {
      // An estimate that could not be made is shown as no estimate. It never blocks
      // anything, so there is nothing to report and nowhere to report it.
      strength = null;
    }
  }

  /** Turns whatever the core refused with into a sentence, without inventing a reason. */
  function explain(cause: unknown): string {
    const error = cause as Partial<VaultError> | null;

    switch (error?.kind) {
      case 'alreadyExists':
        return 'Este equipo ya tiene una caja fuerte. No se crea otra encima.';
      case 'passwordTooShort':
        return `La contraseña necesita al menos ${String(error.min)} caracteres.`;
      case 'passwordTooLong':
        return `La contraseña es demasiado larga: el máximo son ${String(error.max)} bytes.`;
      case 'paramOutOfRange':
        return 'Los parámetros de derivación no son válidos.';
      case 'derivationRefused':
        return 'Este equipo no ha podido ejecutar la derivación con estos parámetros.';
      case 'storage':
        return 'No se ha podido escribir la cabecera. No se ha creado nada.';
      default:
        return 'No se ha podido crear la caja fuerte.';
    }
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!ready) {
      return;
    }

    busy = true;
    problem = null;
    // Copied out and the fields emptied in the same breath, so that what is on screen and
    // what is in the component stop holding the password before the round trip even starts.
    // It never passes through a store: from the field to the core and nowhere else.
    const typed = password;
    password = '';
    repeated = '';
    strength = null;

    try {
      oncreated(await ipc.createVault(typed, INITIAL_PARAMS));
    } catch (cause) {
      problem = explain(cause);
    } finally {
      busy = false;
    }
  }
</script>

<section class="create">
  <div class="column">
    <header>
      <span class="label">Caja fuerte</span>
      <h1>Crear la caja fuerte</h1>
      <div class="rule" aria-hidden="true"></div>
      <p class="lede">
        La contraseña maestra es lo único que abre esta caja fuerte. Elige algo largo que puedas
        recordar sin escribirlo en ningún sitio.
      </p>
    </header>

    <!--
    Before the fields rather than after. A warning underneath a form is a warning somebody
    reads once they have already decided, and this one is not recoverable.
  -->
    <div class="warning" role="note">
      <h2>Esto no se puede deshacer</h2>
      <p>
        No hay frase de recuperación, ni pista, ni segunda puerta, ni forma de que nadie restablezca
        esta contraseña. Si la olvidas, todo lo que guardes aquí queda ilegible para siempre. Es
        deliberado: una puerta de recuperación es una puerta.
      </p>
    </div>

    <form onsubmit={submit}>
      <div class="field">
        <label for="password">Contraseña maestra</label>
        <input
          id="password"
          type={revealed ? 'text' : 'password'}
          bind:value={password}
          oninput={estimate}
          autocomplete="new-password"
          spellcheck="false"
          disabled={busy}
          aria-describedby="password-help"
        />
        <p id="password-help" class="help">
          Al menos {MIN_CHARS} caracteres.
          {#if strength !== null}
            Esta parece <strong>{strengthLabels[strength]}</strong>.
          {/if}
        </p>
        {#if tooShort}
          <p class="problem" role="alert">
            Faltan {MIN_CHARS - chars} caracteres.
          </p>
        {/if}
      </div>

      <div class="field">
        <label for="repeated">Repite la contraseña</label>
        <input
          id="repeated"
          type={revealed ? 'text' : 'password'}
          bind:value={repeated}
          autocomplete="new-password"
          spellcheck="false"
          disabled={busy}
        />
        {#if mismatched}
          <p class="problem" role="alert">Las dos contraseñas no coinciden.</p>
        {/if}
      </div>

      <label class="toggle">
        <input type="checkbox" bind:checked={revealed} disabled={busy} />
        Mostrar lo que escribo
      </label>

      <label class="toggle">
        <input type="checkbox" bind:checked={acknowledged} disabled={busy} />
        Entiendo que si olvido esta contraseña pierdo todo lo que guarde aquí.
      </label>

      <button type="submit" disabled={!ready}>
        {busy ? 'Creando la caja fuerte…' : 'Crear la caja fuerte'}
      </button>
    </form>

    <div role="status" aria-live="polite">
      {#if problem !== null}
        <p class="problem">{problem}</p>
      {:else if busy}
        <p class="help">
          Derivando la clave. Tarda a propósito: es lo que hace cara cada prueba de quien intente
          adivinarla.
        </p>
      {/if}
    </div>
  </div>

  <!-- Its own region of the layout, with nothing on top of it. -->
  <div class="composition">
    <Marks shapes={2} />
  </div>
</section>

<style>
  .create {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-8);
    width: 100%;
    max-width: var(--content-max);
    margin: 0 auto;
  }

  .column {
    display: flex;
    max-width: var(--form-max-width);
    flex-direction: column;
    gap: var(--space-5);
  }

  .composition {
    flex: none;
    padding-top: var(--space-2);
  }

  header {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
  }

  h1 {
    margin-top: var(--space-3);
  }

  .rule {
    width: var(--space-8);
    height: var(--rule-width);
    margin-top: var(--space-4);
    background-color: var(--colour-rule);
  }

  /* The stronger muted colour, not the ordinary one: this screen is held to AAA. */
  .lede {
    margin-top: var(--space-4);
    color: var(--colour-text-muted-strong);
  }

  .warning {
    padding: var(--space-4);
    border: var(--border-width) solid var(--colour-warning);
    border-radius: var(--radius-lg);
    background-color: var(--colour-surface-raised);
  }

  .warning h2 {
    margin: 0 0 var(--space-2);
    color: var(--colour-warning);
    font-size: var(--text-base);
  }

  .warning p {
    margin: 0;
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
    font-weight: var(--weight-semibold);
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
    align-items: flex-start;
    font-weight: var(--weight-regular);
  }

  .help {
    margin: 0;
    color: var(--colour-text-muted-strong);
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
    font-weight: var(--weight-semibold);
  }

  button:disabled {
    background-color: var(--colour-surface-raised);
    border-color: var(--colour-border);
    color: var(--colour-text-faint);
  }
</style>
