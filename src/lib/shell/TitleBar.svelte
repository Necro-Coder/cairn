<script lang="ts">
  /**
   * The top bar of a window that has no system decoration: a drag region and the three
   * window controls.
   *
   * Used twice. The header fills it with the brand, the menu and the state of the vault;
   * the lock screen uses it empty, because a window that cannot be moved or closed until
   * somebody types a password would be a trap.
   *
   * Dragging is a pointer gesture and has no keyboard equivalent here, which is a real
   * cost of drawing our own title bar and is written down as such in
   * `docs/architecture/decisions/0007-undecorated-window.md`. The window can still be
   * resized by its edges, which the platform handles, and closed from the keyboard through
   * the control at the end of this bar.
   */
  import { ipc } from '$ipc';
  import type { Snippet } from 'svelte';

  import WindowControls from './WindowControls.svelte';

  interface Props {
    /** What goes in the bar. Empty on the lock screen, which has only the controls. */
    children?: Snippet | undefined;
    /**
     * Which surface the bar sits on.
     *
     * `raised` is the header of the application, which is a piece of furniture and reads
     * as one. `plain` is the same bar over a screen that fills the window — the lock
     * screen, creating the vault — where a band of a different colour across the top would
     * cut the composition in half.
     */
    tone?: 'raised' | 'plain';
  }

  const { children, tone = 'raised' }: Props = $props();

  /**
   * Whether a press landed on something that is not the bar itself.
   *
   * A button inside the bar has to keep working, and a press that reached it must not also
   * hand the window to the window manager: the drag would swallow the click. Controls opt
   * out by carrying `data-no-drag`, which is checked up the tree rather than on the target,
   * because the press usually lands on the icon inside the button.
   */
  function isControl(target: EventTarget | null): boolean {
    return target instanceof Element && target.closest('[data-no-drag]') !== null;
  }

  function beginDrag(event: PointerEvent): void {
    // Primary button only. A right click belongs to whatever context menu the platform
    // offers, and a middle click should do nothing at all.
    if (event.button !== 0 || isControl(event.target)) {
      return;
    }

    // Nothing is reported if this fails. There is no visible change to undo, and the
    // window simply stays where it was, which is what somebody whose drag did nothing
    // will try again.
    void ipc.startWindowDrag().catch(() => undefined);
  }

  function toggleMaximise(event: MouseEvent): void {
    if (isControl(event.target)) {
      return;
    }

    void ipc.toggleMaximizeWindow().catch(() => undefined);
  }
</script>

<!--
  A `header` with pointer handlers and no keyboard equivalent, which the accessibility
  rules would normally object to. It is deliberate and it is the price of the undecorated
  window: moving a window is a pointer gesture on every platform, and the system gesture it
  replaces has no keyboard equivalent either. Everything inside it is an ordinary control
  and is reachable with `Tab`.
-->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<header
  class="title-bar"
  class:plain={tone === 'plain'}
  onpointerdown={beginDrag}
  ondblclick={toggleMaximise}
>
  {#if children !== undefined}
    {@render children()}
  {/if}

  <WindowControls />
</header>

<style>
  .title-bar {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-4);
    min-height: var(--header-height);
    padding: var(--space-2) var(--space-4);
    background-color: var(--colour-surface-raised);
    border-bottom: var(--border-width) solid var(--colour-border);
  }

  /* Over a screen that fills the window there is nothing to separate it from, and a
   * band of a different colour across the top would cut the composition in half. */
  .plain {
    background-color: transparent;
    border-bottom: 0;
  }
</style>
