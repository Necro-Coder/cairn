<script lang="ts">
  /**
   * Minimise, maximise and close, drawn by the application because the window has no
   * system decoration.
   *
   * Each one is a command of our own rather than an entry in the capability list. The
   * standard route, `core:window:allow-minimize` and its three companions, would add four
   * core APIs to what script injected into the WebView could call; the list today is two
   * permissions that can only listen to events this application emits. The reasoning is in
   * `docs/architecture/decisions/0007-undecorated-window.md`.
   *
   * A control that fails says so where it is and leaves the window alone. There is nowhere
   * else to report it to: this is the title bar.
   */
  import { ipc } from '$ipc';

  import IconWindowClose from '../icons/IconWindowClose.svelte';
  import IconWindowMaximise from '../icons/IconWindowMaximise.svelte';
  import IconWindowMinimise from '../icons/IconWindowMinimise.svelte';
  import type { WindowError } from '../ipc.types';

  /**
   * Whether the window is maximised.
   *
   * Seeded false and kept from what each press answers, rather than asked for. The core
   * returns the state after the change, so the button redraws itself from the result of
   * the press instead of from a second round trip that could disagree with it.
   */
  let maximised = $state(false);

  /** What went wrong with the last press, while there is still something to say about it. */
  let problem = $state<string | null>(null);

  /**
   * Turns whatever the command refused with into a sentence.
   *
   * Two outcomes, and they are worth telling apart. The core answering `unavailable` means
   * the window manager said no and the window is still there, which is something to try
   * again. Anything else means the rejection did not come from our own core at all, so the
   * command boundary is broken and trying again will not help.
   */
  function explain(cause: unknown, what: string): string {
    const error = cause as Partial<WindowError> | null;

    return error?.kind === 'unavailable'
      ? `El sistema no ha dejado ${what} la ventana.`
      : `No se ha podido ${what} la ventana: el núcleo no ha respondido.`;
  }

  async function minimise(): Promise<void> {
    problem = null;
    try {
      await ipc.minimizeWindow();
    } catch (cause) {
      problem = explain(cause, 'minimizar');
    }
  }

  async function toggleMaximise(): Promise<void> {
    problem = null;
    try {
      maximised = await ipc.toggleMaximizeWindow();
    } catch (cause) {
      problem = explain(cause, 'redimensionar');
    }
  }

  async function close(): Promise<void> {
    problem = null;
    try {
      await ipc.closeWindow();
    } catch (cause) {
      // The vault has been closed either way: the core does that first, on purpose. What
      // failed is the window, and the application is still running, so saying so here is
      // the only place left that anybody is looking at.
      problem = `${explain(cause, 'cerrar')} La caja fuerte sí se ha cerrado.`;
    }
  }
</script>

<!--
  Marked so the drag region around it can tell that a press landed on a control rather than
  on the bar itself. Without it, pressing close would also start dragging the window.
-->
<div class="window-controls" data-no-drag>
  {#if problem !== null}
    <!-- Beside the button that failed, because this is the title bar: there is no screen
         above it to report into and no notice worth putting over the content. -->
    <p class="problem" role="alert">{problem}</p>
  {/if}

  <button type="button" onclick={minimise} title="Minimizar">
    <IconWindowMinimise label="Minimizar" />
  </button>
  <button type="button" onclick={toggleMaximise} title={maximised ? 'Restaurar' : 'Maximizar'}>
    <IconWindowMaximise {maximised} label={maximised ? 'Restaurar' : 'Maximizar'} />
  </button>
  <button type="button" class="close" onclick={close} title="Cerrar">
    <IconWindowClose label="Cerrar" />
  </button>
</div>

<style>
  .window-controls {
    display: flex;
    align-items: stretch;
    align-self: stretch;
    /* Flush to the top right corner, the way every other window on this system is. The
     * left margin is automatic so the controls end up on the right even on a bar with
     * nothing else in it, and the header's own padding is cancelled so the targets reach
     * the edge of the screen when the window is maximised, which is where a pointer thrown
     * at the corner lands. */
    margin: calc(-1 * var(--space-2)) calc(-1 * var(--space-4)) calc(-1 * var(--space-2)) auto;
  }

  .window-controls button {
    display: grid;
    place-items: center;
    width: var(--window-control-width);
    border: 0;
    background-color: transparent;
    color: var(--colour-text-muted);
    transition: background-color var(--duration-fast) var(--easing);
  }

  .window-controls button:hover {
    background-color: var(--colour-surface-sunken);
    color: var(--colour-text);
  }

  /* Closing is the one that cannot be undone, so it is the one that looks different under
   * the pointer. Colour is not the only signal: the glyph and the title say it too. */
  .close:hover {
    background-color: var(--colour-negative);
    color: var(--colour-accent-contrast);
  }

  .problem {
    align-self: center;
    max-width: var(--field-max-width);
    margin: 0;
    padding-right: var(--space-3);
    color: var(--colour-negative);
    font-size: var(--text-sm);
  }
</style>
