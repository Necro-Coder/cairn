<script lang="ts">
  /**
   * The top of the application: the brand, the `Abrir` menu, the state of the vault, and
   * the three window controls.
   *
   * It is also the title bar, because the window has no system decoration, so this row is
   * both what the application says about itself and what the window is dragged by.
   *
   * Everything about which sections exist comes from `sections.ts`, and everything about
   * the vault comes from the session. The header decides neither; it draws them.
   */
  import BrandMark from '../icons/BrandMark.svelte';
  import type { Snippet } from 'svelte';

  import OpenMenu from './OpenMenu.svelte';
  import TitleBar from './TitleBar.svelte';
  import VaultState from './VaultState.svelte';
  import type { SectionId } from './sections';

  interface Props {
    /** Called with whatever was chosen in the menu. */
    onopen: (id: SectionId) => void;
    /** Anything the menu offers besides the sections, drawn under a separator. */
    menuExtra?: Snippet<[() => void]> | undefined;
  }

  const { onopen, menuExtra }: Props = $props();
</script>

<TitleBar>
  <!--
    The glyph and the name. The glyph is in full vermilion, which is the one named
    exception to "vermilion means action" in the design system, and it is fenced by two
    rules: it is never next to a primary button and it is never clickable. Both hold here.
  -->
  <span class="brand">
    <BrandMark />
    Cairn
  </span>

  <OpenMenu onchoose={onopen} extra={menuExtra} />

  <span class="spacer"></span>

  <VaultState />
</TitleBar>

<style>
  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: var(--weight-bold);
    letter-spacing: var(--tracking-tight);
    /* Not a control, so it takes no pointer events of its own and a press on it drags the
     * window like a press on any other part of the bar. */
  }

  .spacer {
    flex: 1;
  }
</style>
