<script lang="ts">
  import { ipc } from '$ipc';

  import CheckScreen from './routes/CheckScreen.svelte';
  import CreateVaultScreen from './routes/CreateVaultScreen.svelte';
  import DiagnosticsScreen from './routes/DiagnosticsScreen.svelte';
  import UnlockScreen from './routes/UnlockScreen.svelte';
  import { measureStartup, type StartupResult } from './lib/startup';
  import { session } from './lib/session.svelte';
  import type { VaultStatus } from './lib/ipc.types';

  // Null in every real build, so the banner below is not hidden by a condition: its text
  // does not exist in the bundle at all. The module that invents the data is the one that
  // says so, which is why this comes across the boundary rather than from a build flag.
  const previewNotice = ipc.previewNotice;

  let startup = $state<StartupResult | null>(null);
  let diagnosticsOpen = $state(false);
  let ready = $state(false);

  // Measured as soon as the interface exists, so that the cold start figure is the time
  // the person waited rather than however long they took to press something.
  void measureStartup()
    .then((result) => {
      startup = result;
    })
    .catch(() => {
      // A failure here means the command boundary is broken, and the screen below already
      // reports that in a place the person is looking at. Repeating it would add noise
      // without adding information, and there is nothing to log to.
      startup = null;
    });

  // Read before anything is drawn, because what to draw is entirely this answer: a machine
  // with no vault, a vault that is closed, or one that is open.
  void session
    .refresh()
    .catch(() => undefined)
    .finally(() => {
      ready = true;
    });

  $effect(() => {
    // Started here rather than at module load so that it stops when the interface goes away,
    // which is what keeps a timer from outliving the thing it was reporting for.
    let stop: (() => void) | null = null;
    void session.start().then((cancel) => {
      stop = cancel;
    });

    return () => {
      stop?.();
      session.stop();
    };
  });

  /**
   * How long before the automatic lock the interface warns, in seconds.
   *
   * Kept in step with `WARNING_BEFORE_LOCK_S` in `cairn-domain`, which is what documents the
   * choice. Nothing enforces the agreement, and nothing needs to: a warning that appeared
   * slightly early or late would be a cosmetic mistake rather than a security one.
   */
  const WARNING_BEFORE_LOCK_S = 30;

  /**
   * Whether the vault was open the last time this was looked at.
   *
   * A plain variable rather than state, because the effect below writes it and nothing
   * draws it. Making it reactive would make that effect depend on its own result.
   */
  let wasUnlocked = false;

  // Closing the vault has to take the diagnostics panel with it. The panel is drawn above
  // the router, so without this a lock that happens while it is open leaves it on screen,
  // with whatever was half typed into the change forms still sitting in their fields, and
  // the lock screen never appears. That is the one thing the automatic lock exists to
  // prevent: somebody sitting down at a machine whose owner walked away.
  //
  // Watched as a transition rather than as a condition. A condition would make the panel
  // impossible to open at all while the vault is closed, and reading the version is how
  // anybody works out what is wrong with a machine that will not open.
  $effect(() => {
    const unlocked = session.status.unlocked;

    if (wasUnlocked && !unlocked) {
      diagnosticsOpen = false;
    }

    wasUnlocked = unlocked;
  });

  const closingSoon = $derived(
    session.status.idleRemainingS !== null && session.status.idleRemainingS <= WARNING_BEFORE_LOCK_S
      ? session.status.idleRemainingS
      : null,
  );

  function adopt(status: VaultStatus): void {
    session.adopt(status);
  }

  function handleKeydown(event: KeyboardEvent): void {
    session.noteActivity();

    // Control and Shift together, so that nothing typed by accident opens a screen nobody
    // asked for. The shortcut works in release builds too: the machine where something
    // goes wrong is rarely the one with a debugger attached.
    if (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === 'd') {
      event.preventDefault();
      diagnosticsOpen = !diagnosticsOpen;
      return;
    }

    if (event.key === 'Escape' && diagnosticsOpen) {
      event.preventDefault();
      diagnosticsOpen = false;
    }
  }
</script>

<svelte:window
  onkeydown={handleKeydown}
  onpointerdown={() => session.noteActivity()}
  onpointermove={() => session.noteActivity()}
/>

{#if previewNotice !== null}
  <!--
    Sticky rather than fixed, so it takes its own row instead of covering the first one,
    and with no way to close it. A warning that can be dismissed is a warning that will be,
    two minutes into looking at a screen and an hour before somebody decides something
    based on numbers this build invented.
  -->
  <p class="preview-banner" role="alert">{previewNotice}</p>
{/if}

<main>
  {#if diagnosticsOpen}
    <DiagnosticsScreen {startup} onclose={() => (diagnosticsOpen = false)} />
  {:else if !ready}
    <p class="muted">Leyendo el estado de la caja fuerte…</p>
  {:else if session.status.condition === 'unreadable'}
    <!--
      A header that is there and cannot be read. The one thing this screen must never offer
      is creating a new vault, because that would write over the damaged one and make
      everything encrypted under it unreadable for good.
    -->
    <section class="damaged">
      <h1>La cabecera no se puede leer</h1>
      <p>
        Hay una caja fuerte en este equipo y su cabecera está dañada. No se va a crear otra encima:
        eso dejaría ilegible todo lo que haya guardado.
      </p>
      <p>
        Recupera el fichero <code>vault.header</code> desde tu propia copia de seguridad y vuelve a abrir
        la aplicación.
      </p>
    </section>
  {:else if !session.status.exists}
    <CreateVaultScreen oncreated={adopt} />
  {:else if !session.status.unlocked}
    <UnlockScreen status={session.status} lockReason={session.lockReason} onunlocked={adopt} />
  {:else}
    <CheckScreen />
  {/if}

  {#if session.status.unlocked}
    <!--
      Computed from the countdown the core reports rather than announced by an event of its
      own. The interface already has to count down to draw the number, so an extra crossing
      would buy nothing.
    -->
    {#if closingSoon !== null}
      <p class="closing" role="alert">
        La caja fuerte se cerrará sola en {closingSoon}
        {closingSoon === 1 ? 'segundo' : 'segundos'}. Mueve el ratón o escribe algo para seguir.
      </p>
    {/if}

    <button type="button" class="lock" onclick={() => void session.lock()}>
      Cerrar la caja fuerte
    </button>
  {/if}

  <footer>
    <p>
      <kbd>Ctrl</kbd> + <kbd>Mayús</kbd> + <kbd>D</kbd> abre el diagnóstico.
    </p>
  </footer>
</main>

<style>
  .preview-banner {
    position: sticky;
    top: 0;
    z-index: 1;
    max-width: none;
    margin: 0;
    padding: var(--space-3) var(--space-6);
    border-bottom: var(--border-width) solid var(--colour-border-strong);
    /* The one place in the interface that uses the warning colour as a background. It is
     * meant to be impossible to mistake for part of the application. */
    background-color: var(--colour-warning);
    color: var(--colour-surface);
    font-size: var(--text-sm);
    font-weight: 600;
    text-align: center;
    user-select: text;
  }

  main {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--space-6);
    padding: var(--space-7) var(--space-6) var(--space-5);
  }

  .muted {
    color: var(--colour-text-muted);
  }

  .damaged {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: var(--form-max-width);
    padding: var(--space-5);
    border: var(--border-width) solid var(--colour-negative);
    border-radius: var(--radius-lg);
    background-color: var(--colour-surface-raised);
  }

  .damaged h1 {
    margin: 0;
    color: var(--colour-negative);
  }

  .damaged p {
    margin: 0;
  }

  .closing {
    margin: 0;
    padding: var(--space-3) var(--space-4);
    border: var(--border-width) solid var(--colour-warning);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    font-size: var(--text-sm);
  }

  .lock {
    align-self: flex-start;
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font-size: var(--text-sm);
  }

  footer {
    margin-top: auto;
    padding-top: var(--space-4);
    border-top: var(--border-width) solid var(--colour-border);
    color: var(--colour-text-faint);
    font-size: var(--text-sm);
  }
</style>
