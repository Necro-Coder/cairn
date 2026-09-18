<script lang="ts">
  /**
   * Copies: writing one, checking one, putting one back, and getting data out in the clear.
   *
   * Nothing on this screen takes a path from here, and that is the shape of all of it. The
   * core opens the file dialog itself, in a window the operating system draws; a path chosen
   * in a WebView would be a directory somebody else picked. What comes back is a file name
   * and some numbers, never the folder, because a folder carries the account name.
   *
   * Restoring is two steps because it is the one thing here that destroys data somebody
   * still has. The first reads the whole file into a database of its own and touches nothing;
   * only then, knowing the file is good, does the screen ask the question. Nobody is asked to
   * give up their data until the thing replacing it has proved it is worth it.
   *
   * The readable export is the opposite risk and gets the opposite treatment: a word to type
   * and the master password again, because what it produces has nothing protecting it at all.
   *
   * Every password is copied out and its field emptied before the round trip, so this
   * component is not still holding one while a minute of Argon2id and disk work runs.
   */
  import { ipc } from '$ipc';
  import Badge from '../../lib/shell/Badge.svelte';
  import { sectionOf } from '../../lib/shell/sections';
  import { session } from '../../lib/session.svelte';
  import type {
    BackupError,
    BackupModule,
    BackupPasswordSource,
    ImportPreparedReport,
  } from '../../lib/ipc.types';

  /** Borrowed from settings' own colour, because none of this belongs to a module. */
  const section = sectionOf('settings');

  /** How many bytes go in a mebibyte, for the one place a size is put into words. */
  const BYTES_PER_MIB = 1024 * 1024;

  /**
   * What somebody has to type before a readable export will run.
   *
   * A guard against absent-mindedness and nothing more, which is why it lives here and not in
   * the core. The barrier that actually stops somebody else at the keyboard is the master
   * password, and that one is checked in Rust.
   */
  const CONFIRMATION_WORD = 'EXPORTAR';

  const MODULES: readonly { readonly id: BackupModule; readonly label: string }[] = [
    { id: 'habits', label: 'Hábitos' },
    { id: 'vault', label: 'Caja fuerte de contraseñas' },
    { id: 'finance', label: 'Finanzas' },
  ];

  const OPERATIONS = [
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

  let importPassword = $state('');
  let prepared = $state<ImportPreparedReport | null>(null);

  let plaintextModule = $state<BackupModule>('habits');
  let plaintextWord = $state('');
  let plaintextPassword = $state('');

  let busy = $state<'export' | 'verify' | 'prepare' | 'replace' | 'plaintext' | null>(null);
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

  const importReady = $derived(
    busy === null && unlocked && importPassword.length > 0 && prepared === null,
  );

  const plaintextReady = $derived(
    busy === null &&
      unlocked &&
      plaintextPassword.length > 0 &&
      plaintextWord.trim().toUpperCase() === CONFIRMATION_WORD,
  );

  /** How many rows the prepared file holds altogether, for the sentence that asks. */
  const preparedRows = $derived(
    (prepared?.recordsByTable ?? []).reduce((total, count) => total + count.rows, 0),
  );

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
      case 'alreadyPreparing':
        return 'Ya hay una copia preparada esperando respuesta. Contéstala o descártala antes de leer otra.';
      case 'unknownToken':
        return 'La copia preparada ya no vale: han pasado más de diez minutos o la caja fuerte se ha cerrado. Vuelve a elegir el fichero.';
      case 'restoredButNotOpen':
        // The one message on this screen that must not be softened. It says the opposite of
        // every other failure here: the replacement did happen.
        return 'La copia se ha restaurado y la caja fuerte no se ha podido volver a abrir. Tus datos nuevos están en su sitio y los anteriores están en la copia que se guardó antes. Cierra la aplicación y vuelve a abrirla.';
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
  async function run<T>(
    which: 'export' | 'verify' | 'prepare' | 'replace' | 'plaintext',
    operation: () => Promise<T>,
  ): Promise<T> {
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

  async function submitImport(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!importReady) {
      return;
    }

    const password = importPassword;
    importPassword = '';

    try {
      prepared = await run('prepare', () => ipc.beginImport(password));
    } catch (cause) {
      problem = explain(cause);
    }
  }

  /**
   * Says yes to the prepared copy, which is the one action here that cannot be undone.
   *
   * The token is cleared before the call rather than after. It is single use in the core too,
   * so a second press while the first is running would be refused anyway; clearing it first
   * means the screen never offers a button that is going to be refused.
   */
  async function replaceEverything(): Promise<void> {
    const waiting = prepared;
    if (waiting === null || busy !== null) {
      return;
    }

    prepared = null;

    try {
      const report = await run('replace', () => ipc.commitImport(waiting.token));
      result = `Restaurado. La caja fuerte que tenías se ha guardado antes en ${report.backupCopyFileName}, en la carpeta que elegiste.`;
    } catch (cause) {
      problem = explain(cause);
    }
  }

  async function discardPrepared(): Promise<void> {
    const waiting = prepared;
    if (waiting === null || busy !== null) {
      return;
    }

    prepared = null;

    try {
      await ipc.cancelImport(waiting.token);
      result = 'Descartada. No se ha tocado nada.';
    } catch (cause) {
      problem = explain(cause);
    }
  }

  async function submitPlaintext(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!plaintextReady) {
      return;
    }

    const password = plaintextPassword;
    const chosen = plaintextModule;
    plaintextPassword = '';
    plaintextWord = '';

    try {
      const report = await run('plaintext', () => ipc.exportPlaintext(chosen, password));
      result = `Escrito ${report.fileName} con ${report.records} filas, sin cifrar. Guárdalo donde lo guardarías si estuviera escrito a mano.`;
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

  <section>
    <h2>Restaurar una copia</h2>

    <p class="muted">
      Se lee el fichero entero en una base de datos aparte antes de tocar nada. Si algo falla, falla
      ahí y tu caja fuerte se queda como estaba. Solo cuando la copia está leída y comprobada se te
      pregunta si quieres sustituir lo que tienes.
    </p>

    {#if !unlocked}
      <p class="muted">Abre la caja fuerte para poder restaurar.</p>
    {:else if prepared === null}
      <form onsubmit={submitImport}>
        <label for="import-password">Contraseña de la copia</label>
        <input
          id="import-password"
          type="password"
          bind:value={importPassword}
          autocomplete="off"
          disabled={busy !== null}
        />

        <div class="actions">
          <button type="submit" class="reveal" disabled={!importReady}>
            {busy === 'prepare' ? 'Leyendo la copia…' : 'Elegir un fichero y prepararlo'}
          </button>
        </div>
      </form>
    {:else}
      <p class="warning" role="note">
        {#if prepared.hasExistingData}
          Esto sustituye <strong>todo</strong> lo que hay ahora en esta caja fuerte por lo que trae la
          copia. No se mezcla nada: lo que tienes desaparece. Antes de hacerlo se guarda una copia de
          tu caja fuerte actual y te preguntaremos dónde dejarla.
        {:else}
          Esta caja fuerte está vacía, así que no se pierde nada. Aun así se guarda una copia antes
          de sustituirla y te preguntaremos dónde dejarla.
        {/if}
      </p>

      <p class="muted">
        La copia {prepared.fileName} se ha leído entera y trae {preparedRows} filas.
      </p>

      <ul class="counts">
        {#each prepared.recordsByTable.filter((count) => count.rows > 0) as count (count.table)}
          <li class="count">
            <span>{count.table}</span>
            <span>{count.rows}</span>
          </li>
        {/each}
      </ul>

      <div class="actions">
        <button
          type="button"
          class="destructive"
          onclick={replaceEverything}
          disabled={busy !== null}
        >
          {busy === 'replace' ? 'Sustituyendo…' : 'Sustituir lo que tengo por esta copia'}
        </button>
        <button type="button" class="reveal" onclick={discardPrepared} disabled={busy !== null}>
          Descartar
        </button>
      </div>
    {/if}
  </section>

  <section>
    <h2>Sacar un módulo sin cifrar</h2>

    <p class="muted">
      Un fichero de hoja de cálculo con lo que hay en un módulo, legible por cualquiera. Existe
      porque unos datos de los que no se puede salir son unos datos secuestrados, y va en un solo
      sentido: esto sale, no vuelve a entrar.
    </p>

    {#if !unlocked}
      <p class="muted">Abre la caja fuerte para poder exportar sin cifrar.</p>
    {:else}
      <p class="warning" role="note">
        El fichero que sale de aquí no lo protege nada. Cualquiera que lo abra lo lee entero,
        contraseñas incluidas si eliges la caja fuerte. Quedará anotado en el historial de la
        aplicación que lo hiciste.
      </p>

      <form onsubmit={submitPlaintext}>
        <fieldset>
          <legend>Qué módulo</legend>

          {#each MODULES as option (option.id)}
            <label class="choice">
              <input
                type="radio"
                value={option.id}
                bind:group={plaintextModule}
                disabled={busy !== null}
              />
              {option.label}
            </label>
          {/each}
        </fieldset>

        <label for="plaintext-word">Escribe {CONFIRMATION_WORD} para confirmar</label>
        <input
          id="plaintext-word"
          type="text"
          bind:value={plaintextWord}
          autocomplete="off"
          spellcheck="false"
          disabled={busy !== null}
        />

        <label for="plaintext-password">Contraseña maestra</label>
        <input
          id="plaintext-password"
          type="password"
          bind:value={plaintextPassword}
          autocomplete="current-password"
          disabled={busy !== null}
        />

        <div class="actions">
          <button type="submit" class="destructive" disabled={!plaintextReady}>
            {busy === 'plaintext' ? 'Escribiendo…' : 'Escribir el fichero sin cifrar'}
          </button>
        </div>
      </form>
    {/if}
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

  /*
   * The two buttons that do something nobody can undo. Bordered rather than accented, because
   * the screen already spends its one accent on exporting, and because an action that destroys
   * data should not be the most inviting thing on the page. What marks them is the warning
   * colour, which is the same colour as the block of text above them saying what they do.
   */
  .actions button.destructive {
    padding: var(--space-3) var(--space-5);
    border: var(--border-width) solid var(--colour-warning);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font-weight: var(--weight-semibold);
  }

  .actions button.destructive:disabled {
    border-color: var(--colour-border);
    color: var(--colour-text-faint);
  }

  input[type='text'] {
    padding: var(--space-3);
    border: var(--border-width) solid var(--colour-border);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface);
    color: var(--colour-text);
    font-family: var(--font-mono);
  }

  /* What the prepared copy holds, table by table. A list, so it is separated by rules. */
  ul.counts {
    max-width: var(--field-max-width);
  }

  li.count {
    flex-direction: row;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-2) 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
    font-family: var(--font-mono);
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
