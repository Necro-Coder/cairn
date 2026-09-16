<script lang="ts">
  import { ipc } from '$ipc';
  import { session } from '../lib/session.svelte';
  import type { InactivityChoice, KdfParams, VaultError } from '../lib/ipc.types';

  /** The periods the interface offers, in the order somebody reads them. */
  const PERIODS: readonly { readonly choice: InactivityChoice; readonly label: string }[] = [
    { choice: 'one', label: '1 minuto' },
    { choice: 'five', label: '5 minutos' },
    { choice: 'fifteen', label: '15 minutos' },
    { choice: 'thirty', label: '30 minutos' },
    { choice: 'never', label: 'Nunca' },
  ];

  const status = $derived(session.status);
  const kdf = $derived(status.kdf);

  let inactivityProblem = $state<string | null>(null);

  // The change forms. Kept collapsed, because both are operations somebody should have to
  // decide to start rather than find themselves halfway through.
  let changingPassword = $state(false);
  let changingParams = $state(false);

  let currentPassword = $state('');
  let newPassword = $state('');
  let repeatedPassword = $state('');
  let acknowledgedPassword = $state(false);

  let paramsPassword = $state('');
  let memoryKib = $state(65_536);
  let passes = $state(3);
  let lanes = $state(1);
  let acknowledgedParams = $state(false);

  let busy = $state(false);
  let problem = $state<string | null>(null);
  let done = $state<string | null>(null);

  const passwordReady = $derived(
    !busy &&
      currentPassword.length > 0 &&
      newPassword.length > 0 &&
      newPassword === repeatedPassword &&
      acknowledgedPassword,
  );

  const paramsReady = $derived(!busy && paramsPassword.length > 0 && acknowledgedParams);

  /** Turns whatever the core refused with into a sentence, without inventing a reason. */
  function explain(cause: unknown): string {
    const error = cause as Partial<VaultError> | null;

    switch (error?.kind) {
      case 'noVault':
        return 'Este equipo no tiene ninguna caja fuerte.';
      case 'passwordTooShort':
        return `La contraseña nueva necesita al menos ${String(error.min)} caracteres.`;
      case 'passwordTooLong':
        return `La contraseña es demasiado larga: el máximo son ${String(error.max)} bytes.`;
      case 'paramOutOfRange':
        return `El valor de ${String(error.field)} está fuera del rango permitido, que va de ${String(error.min)} a ${String(error.max)}.`;
      case 'derivationRefused':
        return 'Este equipo no ha podido ejecutar la derivación con esos parámetros. No se ha cambiado nada.';
      case 'storage':
        return 'No se ha podido escribir la cabecera. No se ha cambiado nada y la copia anterior sigue valiendo.';
      default:
        return 'La contraseña actual no es correcta. No se ha cambiado nada.';
    }
  }

  function clearPasswordForm(): void {
    currentPassword = '';
    newPassword = '';
    repeatedPassword = '';
    acknowledgedPassword = false;
  }

  async function submitPassword(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!passwordReady) {
      return;
    }

    busy = true;
    problem = null;
    done = null;
    // Copied out and the fields emptied before the round trip, so neither the screen nor
    // this component is still holding either password while it runs.
    const current = currentPassword;
    const next = newPassword;
    clearPasswordForm();

    try {
      session.adopt(await ipc.changeMasterPassword(current, next));
      changingPassword = false;
      done = 'La contraseña maestra ha cambiado. Nada de lo guardado se ha vuelto a cifrar.';
    } catch (cause) {
      problem = explain(cause);
    } finally {
      busy = false;
    }
  }

  async function submitParams(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!paramsReady) {
      return;
    }

    busy = true;
    problem = null;
    done = null;
    const password = paramsPassword;
    paramsPassword = '';
    acknowledgedParams = false;
    const params: KdfParams = { memoryKib, passes, lanes };

    try {
      session.adopt(await ipc.changeKdfParams(password, params));
      changingParams = false;
      done = 'Los parámetros han cambiado. Ni la contraseña ni un solo byte guardado cambian.';
    } catch (cause) {
      problem = explain(cause);
    } finally {
      busy = false;
    }
  }

  async function chooseInactivity(choice: InactivityChoice): Promise<void> {
    inactivityProblem = null;
    try {
      await session.setInactivity(choice);
    } catch {
      inactivityProblem = 'No se ha podido cambiar el tiempo de inactividad.';
    }
  }

  // Seeded from what the core reports, so the form starts at what is actually in force
  // rather than at a number written into this file.
  $effect(() => {
    if (kdf !== null) {
      memoryKib = kdf.memoryKib;
      passes = kdf.passes;
      lanes = kdf.lanes;
    }
  });
</script>

<div class="panel">
  <section>
    <h3>Derivación de la clave</h3>
    {#if kdf === null}
      <p class="muted">Este equipo todavía no tiene ninguna caja fuerte.</p>
    {:else}
      <dl>
        <div>
          <dt>Memoria</dt>
          <dd>{kdf.memoryKib} KiB</dd>
        </div>
        <div>
          <dt>Pasadas</dt>
          <dd>{kdf.passes}</dd>
        </div>
        <div>
          <dt>Carriles</dt>
          <dd>{kdf.lanes}</dd>
        </div>
      </dl>
    {/if}
  </section>

  <section>
    <h3>Cerrar sola por inactividad</h3>
    <p class="muted">
      Actividad significa teclado o ratón dentro de esta ventana. Lo que hagas en otro programa no
      cuenta.
    </p>
    <div class="periods" role="group" aria-label="Tiempo de inactividad">
      {#each PERIODS as period (period.choice)}
        <button
          type="button"
          class:chosen={status.inactivity === period.choice}
          aria-pressed={status.inactivity === period.choice}
          onclick={() => chooseInactivity(period.choice)}
        >
          {period.label}
        </button>
      {/each}
    </div>

    {#if status.inactivity === 'never'}
      <p class="warning" role="note">
        Con «Nunca» la caja fuerte se queda abierta hasta que la cierres tú, apagues el equipo o
        minimices la ventana. Cualquiera que se siente delante la encuentra abierta.
      </p>
    {/if}
    {#if inactivityProblem !== null}
      <p class="problem" role="alert">{inactivityProblem}</p>
    {/if}
  </section>

  {#if status.exists}
    <section>
      <h3>Cambiar la contraseña maestra</h3>
      {#if !changingPassword}
        <button type="button" class="reveal" onclick={() => (changingPassword = true)}>
          Cambiar la contraseña maestra
        </button>
      {:else}
        <p class="warning" role="note">
          Antes de reescribir la cabecera se guarda una copia al lado y se vuelve a leer para
          comprobar que se puede abrir. Si algo falla, no se cambia nada y la contraseña actual
          sigue valiendo. Aun así, haz tu propia copia del fichero <code>vault.header</code>
          antes de seguir.
        </p>

        <form onsubmit={submitPassword}>
          <label for="current">Contraseña actual</label>
          <input
            id="current"
            type="password"
            bind:value={currentPassword}
            autocomplete="current-password"
            disabled={busy}
          />

          <label for="new">Contraseña nueva</label>
          <input
            id="new"
            type="password"
            bind:value={newPassword}
            autocomplete="new-password"
            disabled={busy}
          />

          <label for="repeat">Repite la nueva</label>
          <input
            id="repeat"
            type="password"
            bind:value={repeatedPassword}
            autocomplete="new-password"
            disabled={busy}
          />

          <label class="toggle">
            <input type="checkbox" bind:checked={acknowledgedPassword} disabled={busy} />
            Tengo una copia del fichero de cabecera y entiendo que si olvido la contraseña nueva lo pierdo
            todo.
          </label>

          <div class="actions">
            <button type="submit" disabled={!passwordReady}>
              {busy ? 'Cambiando…' : 'Cambiar'}
            </button>
            <button
              type="button"
              class="reveal"
              disabled={busy}
              onclick={() => {
                changingPassword = false;
                clearPasswordForm();
              }}
            >
              Cancelar
            </button>
          </div>
        </form>
      {/if}
    </section>

    <section>
      <h3>Cambiar los parámetros</h3>
      <p class="muted">
        Subirlos hace más cara cada prueba de quien intente adivinar la contraseña, y también más
        lenta cada apertura legítima. No se vuelve a cifrar nada: solo cambia la cabecera.
      </p>

      {#if !changingParams}
        <button type="button" class="reveal" onclick={() => (changingParams = true)}>
          Cambiar los parámetros
        </button>
      {:else}
        <p class="warning" role="note">
          Igual que al cambiar la contraseña, se guarda una copia verificada de la cabecera antes de
          reescribirla. Haz también la tuya. Si eliges más memoria de la que este equipo puede
          reservar, la operación se rechaza y no cambia nada.
        </p>

        <form onsubmit={submitParams}>
          <label for="memory">Memoria en KiB</label>
          <input id="memory" type="number" bind:value={memoryKib} min="32768" disabled={busy} />

          <label for="passes">Pasadas</label>
          <input id="passes" type="number" bind:value={passes} min="3" disabled={busy} />

          <label for="lanes">Carriles</label>
          <input id="lanes" type="number" bind:value={lanes} min="1" max="4" disabled={busy} />

          <label for="params-password">Contraseña maestra</label>
          <input
            id="params-password"
            type="password"
            bind:value={paramsPassword}
            autocomplete="current-password"
            disabled={busy}
          />

          <label class="toggle">
            <input type="checkbox" bind:checked={acknowledgedParams} disabled={busy} />
            Tengo una copia del fichero de cabecera.
          </label>

          <div class="actions">
            <button type="submit" disabled={!paramsReady}>
              {busy ? 'Cambiando…' : 'Cambiar'}
            </button>
            <button
              type="button"
              class="reveal"
              disabled={busy}
              onclick={() => {
                changingParams = false;
                paramsPassword = '';
                acknowledgedParams = false;
              }}
            >
              Cancelar
            </button>
          </div>
        </form>
      {/if}
    </section>
  {/if}

  <div role="status" aria-live="polite">
    {#if problem !== null}
      <p class="problem">{problem}</p>
    {:else if done !== null}
      <p class="done">{done}</p>
    {/if}
  </div>
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding-top: var(--space-4);
    border-top: var(--border-width) solid var(--colour-border);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  h3 {
    margin: 0;
  }

  dl {
    display: grid;
    gap: var(--space-3);
    margin: 0;
  }

  dl div {
    display: grid;
    grid-template-columns: 12rem 1fr;
    gap: var(--space-3);
    align-items: baseline;
  }

  dt {
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  dd {
    margin: 0;
    font-family: var(--font-mono);
    user-select: text;
  }

  .periods {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .periods button {
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font-size: var(--text-sm);
  }

  /* The chosen one is filled as well as outlined, so colour is never the only signal and
   * `aria-pressed` carries it for anything that is not looking at the colour at all. */
  .periods button.chosen {
    border-color: var(--colour-accent);
    background-color: var(--colour-accent);
    color: var(--colour-accent-contrast);
    font-weight: 600;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: 28rem;
  }

  label {
    font-size: var(--text-sm);
    font-weight: 600;
  }

  input[type='password'],
  input[type='number'] {
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
    margin-top: var(--space-2);
    font-weight: 400;
  }

  .actions {
    display: flex;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }

  .actions button[type='submit'] {
    padding: var(--space-3) var(--space-5);
    border: var(--border-width) solid transparent;
    border-radius: var(--radius-md);
    background-color: var(--colour-accent);
    color: var(--colour-accent-contrast);
    font-weight: 600;
  }

  .actions button[type='submit']:disabled {
    background-color: var(--colour-surface-raised);
    border-color: var(--colour-border);
    color: var(--colour-text-faint);
  }

  .reveal {
    align-self: flex-start;
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font-size: var(--text-sm);
  }

  .warning {
    margin: 0;
    padding: var(--space-4);
    border: var(--border-width) solid var(--colour-warning);
    border-radius: var(--radius-lg);
    background-color: var(--colour-surface-raised);
  }

  .muted {
    margin: 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .problem {
    margin: 0;
    color: var(--colour-negative);
  }

  .done {
    margin: 0;
    color: var(--colour-positive);
  }
</style>
