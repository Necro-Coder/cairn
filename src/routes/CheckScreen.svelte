<script lang="ts">
  import { fetchAppInfo, type AppInfo } from '../lib/ipc';

  type Status =
    | { readonly kind: 'idle' }
    | { readonly kind: 'loading' }
    | { readonly kind: 'ready'; readonly appInfo: AppInfo }
    | { readonly kind: 'failed'; readonly message: string };

  let status = $state<Status>({ kind: 'idle' });

  async function check(): Promise<void> {
    status = { kind: 'loading' };
    try {
      status = { kind: 'ready', appInfo: await fetchAppInfo() };
    } catch (cause) {
      // The core answering with an error is the only failure this screen can have, and
      // it means the boundary itself is broken. Saying so is more useful than a spinner
      // that never stops.
      status = {
        kind: 'failed',
        message: cause instanceof Error ? cause.message : String(cause),
      };
    }
  }
</script>

<section class="check">
  <header>
    <h1>Cairn</h1>
    <p class="lede">
      Todavía no hay nada que guardar aquí. Esta pantalla solo comprueba que la interfaz y el núcleo
      se están hablando.
    </p>
  </header>

  <button type="button" onclick={check} disabled={status.kind === 'loading'}>
    {status.kind === 'loading' ? 'Preguntando al núcleo…' : 'Preguntar al núcleo'}
  </button>

  <div class="result" role="status" aria-live="polite">
    {#if status.kind === 'idle'}
      <p class="muted">Pulsa el botón para llamar al núcleo en Rust.</p>
    {:else if status.kind === 'loading'}
      <p class="muted">Esperando respuesta…</p>
    {:else if status.kind === 'failed'}
      <p class="error">
        El núcleo no ha respondido: {status.message}
      </p>
    {:else}
      <dl>
        <div>
          <dt>Aplicación</dt>
          <dd>{status.appInfo.name}</dd>
        </div>
        <div>
          <dt>Versión</dt>
          <dd>{status.appInfo.version}</dd>
        </div>
        <div>
          <dt>Compilación</dt>
          <dd>{status.appInfo.profile}</dd>
        </div>
      </dl>
    {/if}
  </div>
</section>

<style>
  .check {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    align-items: flex-start;
  }

  header {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .lede {
    color: var(--colour-text-muted);
  }

  button {
    padding: var(--space-3) var(--space-5);
    border: var(--border-width) solid transparent;
    border-radius: var(--radius-md);
    background-color: var(--colour-accent);
    color: var(--colour-accent-contrast);
    font-weight: 600;
    transition: background-color var(--duration-fast) var(--easing);
  }

  button:hover:not(:disabled) {
    background-color: var(--colour-accent-strong);
  }

  button:disabled {
    background-color: var(--colour-surface-raised);
    border-color: var(--colour-border);
    color: var(--colour-text-faint);
    cursor: progress;
  }

  .result {
    width: 100%;
    min-height: 6.5rem;
    padding: var(--space-4);
    border: var(--border-width) solid var(--colour-border);
    border-radius: var(--radius-lg);
    background-color: var(--colour-surface-raised);
  }

  .muted {
    color: var(--colour-text-muted);
  }

  .error {
    color: var(--colour-negative);
  }

  dl {
    display: grid;
    gap: var(--space-3);
    margin: 0;
  }

  dl div {
    display: grid;
    grid-template-columns: 9rem 1fr;
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
</style>
