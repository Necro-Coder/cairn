<script lang="ts">
  import { ipc } from '$ipc';

  import CheckScreen from './routes/CheckScreen.svelte';
  import DiagnosticsScreen from './routes/DiagnosticsScreen.svelte';
  import { measureStartup, type StartupResult } from './lib/startup';

  // Null in every real build, so the banner below is not hidden by a condition: its text
  // does not exist in the bundle at all. The module that invents the data is the one that
  // says so, which is why this comes across the boundary rather than from a build flag.
  const previewNotice = ipc.previewNotice;

  let startup = $state<StartupResult | null>(null);
  let diagnosticsOpen = $state(false);

  // Measured as soon as the interface exists, so that the cold start figure is the time
  // the person waited rather than however long they took to press something.
  void measureStartup()
    .then((result) => {
      startup = result;
    })
    .catch(() => {
      // A failure here means the command boundary is broken, and the check screen already
      // reports that in a place the person is looking at. Repeating it would add noise
      // without adding information, and there is nothing to log to.
      startup = null;
    });

  function handleKeydown(event: KeyboardEvent): void {
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

<svelte:window onkeydown={handleKeydown} />

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
  {:else}
    <CheckScreen />
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

  footer {
    margin-top: auto;
    padding-top: var(--space-4);
    border-top: var(--border-width) solid var(--colour-border);
    color: var(--colour-text-faint);
    font-size: var(--text-sm);
  }
</style>
