<script lang="ts">
  import { ipc } from '$ipc';
  import type { Diagnostics } from '../lib/ipc.types';
  import { isWithinBudget, type StartupResult } from '../lib/startup';

  interface Props {
    /** Measurements taken on the check screen, if it has run. */
    startup: StartupResult | null;
    /** Closes the screen. Also bound to Escape by the parent. */
    onclose: () => void;
  }

  const { startup, onclose }: Props = $props();

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
    return minutes > 0 ? `${minutes} min ${seconds} s` : `${seconds} s`;
  }

  const databaseLabels: Record<Diagnostics['database'], string> = {
    notInitialized: 'sin inicializar',
  };

  void load();
</script>

<section class="diagnostics">
  <header>
    <div>
      <h2>Diagnóstico</h2>
      <p class="lede">
        Nada de lo que hay aquí identifica al equipo ni a la persona. No aparecen rutas, ni nombre
        de usuario, ni nada que salga de la base de datos.
      </p>
    </div>
    <button type="button" class="close" onclick={onclose}>Cerrar</button>
  </header>

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

  <div class="budgets">
    <h3>Presupuestos</h3>
    {#if startup === null}
      <p class="muted">
        Sin medir todavía. Vuelve a la pantalla anterior y pulsa el botón para tomar la medida.
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
  </div>
</section>

<style>
  .diagnostics {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  header {
    display: flex;
    gap: var(--space-4);
    align-items: flex-start;
    justify-content: space-between;
  }

  header > div {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .lede {
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .close {
    flex-shrink: 0;
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font-size: var(--text-sm);
    transition: border-color var(--duration-fast) var(--easing);
  }

  .close:hover {
    border-color: var(--colour-accent);
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

  .budgets {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding-top: var(--space-4);
    border-top: var(--border-width) solid var(--colour-border);
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
    color: var(--colour-text-muted);
  }

  .error {
    color: var(--colour-negative);
  }
</style>
