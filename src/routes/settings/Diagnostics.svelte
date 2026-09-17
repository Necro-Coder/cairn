<script lang="ts">
  /**
   * What this build is and how it is behaving.
   *
   * Drawn in two places: as the fourth part of the settings screen, and on its own when the
   * vault is closed. The second is not an afterthought — the machine somebody needs this on
   * is usually the one that will not open — so the content is here, once, and the two places
   * are frames around it.
   *
   * Nothing here identifies the machine or the person. No paths, no user name, nothing out of
   * the database. That is a property of what the core answers with, and it is written on the
   * screen so that a change to it is a change somebody has to make against a promise.
   */
  import { ipc } from '$ipc';
  import type { Diagnostics, SampleHabit } from '../../lib/ipc.types';
  import { isWithinBudget, type StartupResult } from '../../lib/startup';

  interface Props {
    /** What the startup measurement found, when it has run. */
    startup: StartupResult | null;
  }

  const { startup }: Props = $props();

  type Status =
    | { readonly kind: 'loading' }
    | { readonly kind: 'ready'; readonly snapshot: Diagnostics }
    | { readonly kind: 'failed'; readonly message: string };

  let status = $state<Status>({ kind: 'loading' });

  /** Reads the snapshot again, replacing what is on screen with what came back. */
  async function refresh(): Promise<void> {
    try {
      status = { kind: 'ready', snapshot: await ipc.fetchDiagnostics() };
    } catch (cause) {
      status = {
        kind: 'failed',
        message: cause instanceof Error ? cause.message : String(cause),
      };
    }
  }

  /**
   * Reads it for the first time, saying so while it is being read.
   *
   * Separate from {@link refresh} rather than a flag on it, and the difference matters on
   * screen: a later read that blanked the panel back to «Leyendo…» would make it flash every
   * time a sample row was written.
   */
  async function load(): Promise<void> {
    status = { kind: 'loading' };
    await refresh();
  }

  function formatUptime(milliseconds: number): string {
    const totalSeconds = Math.floor(milliseconds / 1000);
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return minutes > 0 ? `${String(minutes)} min ${String(seconds)} s` : `${String(seconds)} s`;
  }

  /**
   * What the database state says on screen.
   *
   * A function rather than a lookup table, because three of the four states carry numbers, and
   * a number nobody shows is a number nobody can act on: «abierta» says nothing about a file
   * that has grown a hundred thousand lápidas.
   */
  function describeDatabase(database: Diagnostics['database']): string {
    switch (database.state) {
      case 'notInitialized':
        return 'sin inicializar';
      case 'locked':
        return 'cerrada';
      case 'open':
        return `abierta · esquema ${String(database.schemaVersion)} · ${String(database.tombstones)} lápidas`;
      case 'unsupported':
        return `no compatible · el fichero está en el esquema ${String(database.found)} y esta versión llega al ${String(database.expected)}`;
    }
  }

  /**
   * The sample habits, for the one test that exercises the whole path.
   *
   * They are written to a real table on purpose. A row that survives closing the vault,
   * restarting the application and opening it again is the only evidence that the window, the
   * command boundary, the encryption and the file all work together, and a table invented for
   * testing would only prove that a table invented for testing works.
   */
  let sampleHabits = $state<readonly SampleHabit[]>([]);

  /** What went wrong with the last sample operation, when something did. */
  let sampleProblem = $state<string | null>(null);

  /** True while a sample operation is in flight, so nothing is asked for twice. */
  let sampleBusy = $state(false);

  /** How many rows one page of the sample list holds. */
  const SAMPLE_PAGE = 25;

  /** Whether the vault is open, which is the only state these four commands work in. */
  const databaseIsOpen = $derived(
    status.kind === 'ready' && status.snapshot.database.state === 'open',
  );

  /** Turns whatever a rejected command threw into one sentence. */
  function describeProblem(cause: unknown): string {
    if (typeof cause === 'object' && cause !== null && 'kind' in cause) {
      const { kind } = cause as { kind: string };
      switch (kind) {
        case 'locked':
          return 'La caja está cerrada.';
        case 'notFound':
          return 'Ese hábito ya no está.';
        case 'tooMany':
          return 'Se ha pedido más de lo que admite una llamada.';
        default:
          return 'El almacenamiento no ha podido completar la operación.';
      }
    }
    return cause instanceof Error ? cause.message : String(cause);
  }

  /** Runs one sample operation, keeping the list and the message in step with it. */
  async function runSample(operation: () => Promise<unknown>): Promise<void> {
    if (sampleBusy) {
      return;
    }
    sampleBusy = true;
    sampleProblem = null;
    try {
      await operation();
      sampleHabits = await ipc.listSampleHabits({ after: null, limit: SAMPLE_PAGE });
      // The count of lápidas on the panel above is now out of date, and it is the number this
      // whole section exists to move.
      await refresh();
    } catch (cause) {
      sampleProblem = describeProblem(cause);
    } finally {
      sampleBusy = false;
    }
  }

  /** Whether the list has been read once since the panel was drawn. */
  let sampleListRead = false;

  // Read once, as soon as the panel finds the database open. Somebody who restarted the
  // application to check that a row survived would otherwise be shown an empty list and have to
  // work out which button proves it wrong, which is the opposite of what this section is for.
  $effect(() => {
    if (databaseIsOpen && !sampleListRead) {
      sampleListRead = true;
      void runSample(() => Promise.resolve());
    }
  });

  void load();
</script>

<section>
  <h2>Este equipo</h2>
  <p class="muted">
    Nada de lo que hay aquí identifica al equipo ni a la persona. No aparecen rutas, ni nombre de
    usuario, ni nada que salga de la base de datos.
  </p>

  <div role="status" aria-live="polite">
    {#if status.kind === 'loading'}
      <p class="muted">Leyendo el estado…</p>
    {:else if status.kind === 'failed'}
      <p class="error">No se ha podido leer el estado: {status.message}</p>
    {:else}
      <dl>
        <div>
          <dt>Versión</dt>
          <dd>{status.snapshot.app.version}</dd>
        </div>
        <div>
          <dt>Compilación</dt>
          <dd>{status.snapshot.app.profile}</dd>
        </div>
        <div>
          <dt>Sistema</dt>
          <dd>{status.snapshot.os} · {status.snapshot.arch}</dd>
        </div>
        <div>
          <dt>WebView</dt>
          <dd>{status.snapshot.webviewVersion ?? 'desconocida'}</dd>
        </div>
        <div>
          <dt>Base de datos</dt>
          <dd>{describeDatabase(status.snapshot.database)}</dd>
        </div>
        <div>
          <dt>En marcha desde hace</dt>
          <dd>{formatUptime(status.snapshot.uptimeMs)}</dd>
        </div>
      </dl>
    {/if}
  </div>
</section>

{#if databaseIsOpen}
  <section class="samples">
    <h2>Prueba de almacenamiento</h2>
    <p class="muted">
      Escribe hábitos de prueba en la base real. Al borrar uno queda la fila marcada y su nota se
      vacía, así que el número de lápidas de arriba sube y el contenido no se queda dentro.
    </p>

    <div class="actions">
      <button
        type="button"
        disabled={sampleBusy}
        onclick={() => void runSample(ipc.insertSampleHabit)}
      >
        Añadir uno
      </button>
      <button
        type="button"
        disabled={sampleBusy}
        onclick={() =>
          void runSample(() => ipc.listSampleHabits({ after: null, limit: SAMPLE_PAGE }))}
      >
        Volver a leer
      </button>
    </div>

    <div role="status" aria-live="polite">
      {#if sampleProblem !== null}
        <p class="error">{sampleProblem}</p>
      {:else if sampleHabits.length === 0}
        <p class="muted">Ninguno todavía.</p>
      {:else}
        <ul>
          {#each sampleHabits as habit (habit.id)}
            <li>
              <span class="identifier">{habit.id}</span>
              <button
                type="button"
                disabled={sampleBusy}
                onclick={() => void runSample(() => ipc.deleteSampleHabit(habit.id))}
              >
                Borrar
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </section>
{/if}

<section class="budgets">
  <h2>Presupuestos</h2>
  {#if startup === null}
    <p class="muted">
      Sin medir todavía. La medida se toma sola al arrancar; si no hay número, el arranque no llegó
      a terminar de medirse.
    </p>
  {:else}
    <dl>
      <div>
        <dt>Arranque en frío</dt>
        <dd class:over={!isWithinBudget(startup.coldStart)}>
          {startup.coldStart.milliseconds} ms
          <span class="budget">de {startup.coldStart.budgetMilliseconds} ms</span>
        </dd>
      </div>
      <div>
        <dt>Latencia de un comando</dt>
        <dd class:over={!isWithinBudget(startup.commandLatency)}>
          {startup.commandLatency.milliseconds} ms
          <span class="budget">de {startup.commandLatency.budgetMilliseconds} ms</span>
        </dd>
      </div>
    </dl>
  {/if}
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  /* Sized down from the display scale: the element carries the hierarchy, the size only says
   * how loud it is, and a thirty pixel subheading inside a panel is shouting. */
  h2 {
    margin: 0;
    font-size: var(--text-lg);
    letter-spacing: normal;
  }

  .samples,
  .budgets {
    margin-top: var(--space-5);
    padding-top: var(--space-4);
    border-top: var(--border-width) solid var(--colour-border);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .actions button {
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font-size: var(--text-sm);
  }

  .actions button:disabled {
    border-color: var(--colour-border);
    color: var(--colour-text-faint);
  }

  ul {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--colour-border);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
  }

  /* The identifier is the only thing on screen somebody has to be able to copy out of here and
   * compare with a row in the file, so it is monospaced and selectable like the values above. */
  .identifier {
    overflow-wrap: anywhere;
    color: var(--colour-text-muted);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    user-select: text;
  }

  li button {
    flex-shrink: 0;
    padding: var(--space-1) var(--space-3);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: var(--colour-surface);
    color: var(--colour-text);
    font-size: var(--text-sm);
  }

  li button:disabled {
    border-color: var(--colour-border);
    color: var(--colour-text-faint);
  }

  dl {
    display: grid;
    gap: var(--space-3);
    margin: 0;
  }

  dl div {
    display: grid;
    grid-template-columns: var(--definition-label-width) 1fr;
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

  .budget {
    color: var(--colour-text-faint);
    font-size: var(--text-sm);
  }

  /* Colour is never the only signal, so the budget it was held to stays next to it. */
  .over {
    color: var(--colour-warning);
  }

  .muted {
    max-width: var(--measure);
    margin: 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .error {
    margin: 0;
    color: var(--colour-negative);
  }
</style>
