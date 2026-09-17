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
  import type { Diagnostics } from '../../lib/ipc.types';
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

  async function load(): Promise<void> {
    status = { kind: 'loading' };
    try {
      status = { kind: 'ready', snapshot: await ipc.fetchDiagnostics() };
    } catch (cause) {
      status = {
        kind: 'failed',
        message: cause instanceof Error ? cause.message : String(cause),
      };
    }
  }

  function formatUptime(milliseconds: number): string {
    const totalSeconds = Math.floor(milliseconds / 1000);
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return minutes > 0 ? `${String(minutes)} min ${String(seconds)} s` : `${String(seconds)} s`;
  }

  const databaseLabels: Record<Diagnostics['database'], string> = {
    notInitialized: 'sin inicializar',
  };

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
          <dd>{databaseLabels[status.snapshot.database]}</dd>
        </div>
        <div>
          <dt>En marcha desde hace</dt>
          <dd>{formatUptime(status.snapshot.uptimeMs)}</dd>
        </div>
      </dl>
    {/if}
  </div>
</section>

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

  .budgets {
    margin-top: var(--space-5);
    padding-top: var(--space-4);
    border-top: var(--border-width) solid var(--colour-border);
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
