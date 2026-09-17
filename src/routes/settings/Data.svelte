<script lang="ts">
  /**
   * Copies, exporting and importing.
   *
   * Two of these work now: writing a backup, and reading one back to check it. Importing is
   * still declared and still switched off, because the question "can I get my data out of
   * this?" is one somebody asks before they put anything in, and because the honest answer
   * about restoring is that it is not built yet rather than that it does not exist.
   *
   * Neither operation takes a path from here, and that is the shape of the whole screen. The
   * core opens the file dialog itself, in a window the operating system draws; a path chosen
   * in a WebView would be a directory somebody else picked. What comes back is a file name
   * and some numbers, never the folder, because a folder carries the account name.
   *
   * The password is copied out and the field emptied before the round trip, so this
   * component is not still holding one while a minute of Argon2id and disk work runs.
   */
  import { ipc } from '$ipc';
  import Badge from '../../lib/shell/Badge.svelte';
  import { sectionOf } from '../../lib/shell/sections';
  import { session } from '../../lib/session.svelte';
  import type { BackupError, BackupPasswordSource } from '../../lib/ipc.types';

  /** Borrowed from settings' own colour, because none of this belongs to a module. */
  const section = sectionOf('settings');

  /** How many bytes go in a mebibyte, for the one place a size is put into words. */
  const BYTES_PER_MIB = 1024 * 1024;

  const OPERATIONS = [
    {
      title: 'Importar una copia',
      detail:
        'Leer un fichero exportado desde este u otro equipo, comprobarlo entero antes de tocar nada y sustituir lo que haya. Llega en la segunda mitad de esta fase.',
    },
    {
      title: 'Copia de seguridad de la cabecera',
      detail:
        'La cabecera es lo único sin lo cual no se puede abrir nada, y hoy se copia a mano desde la carpeta de la aplicación. Aquí habrá un botón.',
    },
  ];

  const unlocked = $derived(session.status.unlocked);

  let source = $state<BackupPasswordSource>('master');
  let exportPassword = $state('');
  let repeatedPassword = $state('');
  let acknowledged = $state(false);

  let verifyPassword = $state('');

  let busy = $state<'export' | 'verify' | null>(null);
  let done = $state(0);
  let problem = $state<string | null>(null);
  let result = $state<string | null>(null);

  const exportReady = $derived(
    busy === null &&
      unlocked &&
      exportPassword.length > 0 &&
      acknowledged &&
      (source === 'master' || exportPassword === repeatedPassword),
  );

  const verifyReady = $derived(busy === null && verifyPassword.length > 0);

  /** Turns whatever the core refused with into a sentence, without inventing a reason. */
  function explain(cause: unknown): string | null {
    const error = cause as Partial<BackupError> | null;

    switch (error?.kind) {
      case 'cancelled':
        // Not a failure and not worth a red line. Somebody closed the dialog on purpose.
        return null;
      case 'locked':
        return 'La caja fuerte está cerrada. Ábrela y vuelve a intentarlo.';
      case 'wrongPassword':
        return 'Esa no es la contraseña maestra de este equipo. No se ha escrito nada.';
      case 'lockedOut':
        return `Demasiados intentos con la contraseña maestra. Espera ${String(error.remainingS ?? 0)} segundos. Es la misma cuenta atrás que en la pantalla de apertura, porque es la misma contraseña.`;
      case 'weakPassword':
        return `La contraseña del fichero necesita al menos ${String(error.min ?? 12)} caracteres.`;
      case 'notABackup':
        return 'Ese fichero no es una copia de Cairn.';
      case 'unsupportedVersion':
        return 'Esa copia la escribió una versión más nueva de Cairn. Actualiza antes de leerla.';
      case 'badPassword':
        return 'La copia no se abre con esa contraseña.';
      case 'damaged':
        return 'La copia está dañada o incompleta. No se puede confiar en ella.';
      case 'io':
        return 'No se ha podido leer o escribir el fichero. No se ha dejado nada a medias.';
      default:
        return 'La operación no se ha podido completar. No se ha dejado nada a medias.';
    }
  }

  /** A size in words, to one decimal, so a report reads rather than counts. */
  function inMib(bytes: number): string {
    return `${(bytes / BYTES_PER_MIB).toFixed(1)} MiB`;
  }

  function clearExportForm(): void {
    exportPassword = '';
    repeatedPassword = '';
    acknowledged = false;
  }

  /**
   * Runs one of the two operations, watching its progress while it goes.
   *
   * The subscription is taken before the call and dropped in every path out of it, including
   * the failing one. A listener that outlived its operation would keep moving a bar that
   * belongs to nothing.
   */
  async function run<T>(which: 'export' | 'verify', operation: () => Promise<T>): Promise<T> {
    const stop = await ipc.onBackupProgress((progress) => {
      done = progress.done;
    });

    busy = which;
    done = 0;
    problem = null;
    result = null;

    try {
      return await operation();
    } finally {
      stop();
      busy = null;
    }
  }

  async function submitExport(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!exportReady) {
      return;
    }

    // Copied out and the fields emptied before the round trip, so the screen is not still
    // holding a password while the export runs.
    const password = exportPassword;
    const chosen = source;
    clearExportForm();

    try {
      const report = await run('export', () => ipc.exportBackup(password, chosen));
      result = `Copia escrita y comprobada: ${report.fileName}, ${inMib(report.bytes)}, ${report.records} registros. Se ha vuelto a abrir entera antes de decirte esto.`;
    } catch (cause) {
      problem = explain(cause);
    }
  }

  async function submitVerify(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!verifyReady) {
      return;
    }

    const password = verifyPassword;
    verifyPassword = '';

    try {
      const report = await run('verify', () => ipc.verifyBackup(password));
      result = `La copia ${report.fileName} se abre entera: ${inMib(report.bytes)}, ${report.records} registros, formato versión ${report.formatVersion}.`;
    } catch (cause) {
      problem = explain(cause);
    }
  }
</script>

<div class="panel">
  <section>
    <h2>Exportar una copia</h2>

    <p class="muted">
      Un único fichero cifrado con todo lo guardado. Se escribe primero con otro nombre, se manda al
      disco de verdad, se renombra, y entonces se vuelve a abrir y se descifra entero con la
      contraseña antes de darlo por bueno. Una copia que nadie ha leído nunca no es una copia.
    </p>

    {#if !unlocked}
      <p class="muted">Abre la caja fuerte para poder exportar.</p>
    {:else}
      <p class="warning" role="note">
        Dentro del fichero, una vez descifrado, las contraseñas están en claro. Lo único que lo
        protege es la contraseña con la que lo cifres. Si la olvidas, el fichero no sirve para nada
        y nadie puede recuperarlo, ni tú ni nosotros.
      </p>

      <form onsubmit={submitExport}>
        <fieldset>
          <legend>Contraseña del fichero</legend>

          <label class="choice">
            <input type="radio" value="master" bind:group={source} disabled={busy !== null} />
            La misma contraseña maestra de este equipo
          </label>

          <label class="choice">
            <input type="radio" value="separate" bind:group={source} disabled={busy !== null} />
            Una contraseña distinta, solo para este fichero
          </label>
        </fieldset>

        <label for="export-password">
          {source === 'master' ? 'Contraseña maestra' : 'Contraseña del fichero'}
        </label>
        <input
          id="export-password"
          type="password"
          bind:value={exportPassword}
          autocomplete={source === 'master' ? 'current-password' : 'new-password'}
          disabled={busy !== null}
        />

        {#if source === 'separate'}
          <label for="export-repeat">Repite la contraseña del fichero</label>
          <input
            id="export-repeat"
            type="password"
            bind:value={repeatedPassword}
            autocomplete="new-password"
            disabled={busy !== null}
          />
        {/if}

        <label class="toggle">
          <input type="checkbox" bind:checked={acknowledged} disabled={busy !== null} />
          Entiendo que si pierdo esta contraseña el fichero no se puede abrir.
        </label>

        <div class="actions">
          <button type="submit" disabled={!exportReady}>
            {busy === 'export' ? 'Exportando…' : 'Exportar una copia'}
          </button>
        </div>
      </form>
    {/if}
  </section>

  <section>
    <h2>Comprobar una copia</h2>

    <p class="muted">
      Lee un fichero de copia de principio a fin y dice si se abre entero. No escribe nada y no hace
      falta tener la caja fuerte abierta, que es justo lo que quieres el día que no puedes entrar.
    </p>

    <form onsubmit={submitVerify}>
      <label for="verify-password">Contraseña de la copia</label>
      <input
        id="verify-password"
        type="password"
        bind:value={verifyPassword}
        autocomplete="off"
        disabled={busy !== null}
      />

      <div class="actions">
        <button type="submit" class="reveal" disabled={!verifyReady}>
          {busy === 'verify' ? 'Comprobando…' : 'Elegir un fichero y comprobarlo'}
        </button>
      </div>
    </form>
  </section>

  <div role="status" aria-live="polite">
    {#if busy !== null && done > 0}
      <p class="muted">Procesados {inMib(done)}.</p>
    {:else if problem !== null}
      <p class="problem">{problem}</p>
    {:else if result !== null}
      <p class="done">{result}</p>
    {/if}
  </div>

  <section>
    <h2>Lo que todavía no está</h2>

    <ul>
      {#each OPERATIONS as operation (operation.title)}
        <li>
          <div class="head">
            <button type="button" disabled title="Todavía no: {operation.detail}">
              {operation.title}
            </button>
            <Badge tone={section.tone} text="En desarrollo" />
          </div>
          <p class="muted">{operation.detail}</p>
        </li>
      {/each}
    </ul>
  </section>
</div>

<style>
  /* No rule of its own at the top: the settings screen already draws one under its chooser. */
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  h2 {
    margin: 0;
    font-size: var(--text-lg);
    letter-spacing: normal;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: var(--field-max-width);
  }

  fieldset {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: var(--space-3);
    border: var(--border-width) solid var(--colour-border);
    border-radius: var(--radius-md);
  }

  legend {
    padding: 0 var(--space-2);
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
  }

  label {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
  }

  .choice,
  .toggle {
    display: flex;
    gap: var(--space-3);
    align-items: flex-start;
    font-weight: var(--weight-regular);
  }

  .toggle {
    margin-top: var(--space-2);
  }

  input[type='password'] {
    padding: var(--space-3);
    border: var(--border-width) solid var(--colour-border);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface);
    color: var(--colour-text);
    font-family: var(--font-mono);
  }

  .actions {
    display: flex;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }

  /* The one accented button on this screen, as the design system allows exactly one. */
  .actions button[type='submit'] {
    padding: var(--space-3) var(--space-5);
    border: var(--border-width) solid transparent;
    border-radius: var(--radius-md);
    background-color: var(--colour-accent);
    color: var(--colour-accent-contrast);
    font-weight: var(--weight-semibold);
  }

  .actions button[type='submit']:disabled {
    border-color: var(--colour-border);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text-faint);
  }

  /* Checking a copy is the second action here, so it is bordered rather than accented. */
  .actions button.reveal {
    border: var(--border-width) solid var(--colour-border-strong);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font-weight: var(--weight-regular);
  }

  ul {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* Separated by a rule rather than by alternating background, like every list here. */
  li {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4) 0;
    border-top: var(--border-width) solid var(--colour-border);
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  li button {
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font-size: var(--text-sm);
  }

  li button:disabled {
    border-color: var(--colour-border);
    color: var(--colour-text-faint);
  }

  .warning {
    margin: 0;
    padding: var(--space-4);
    border: var(--border-width) solid var(--colour-warning);
    border-radius: var(--radius-lg);
    background-color: var(--colour-surface-raised);
  }

  .muted {
    max-width: var(--measure);
    margin: 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .problem {
    max-width: var(--measure);
    margin: 0;
    color: var(--colour-negative);
  }

  .done {
    max-width: var(--measure);
    margin: 0;
    color: var(--colour-positive);
  }
</style>
