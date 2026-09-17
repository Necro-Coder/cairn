<script lang="ts">
  /**
   * The frame every icon in this application is drawn in.
   *
   * Eighteen icons share one box, one stroke weight and one decision about assistive
   * technology. Repeating those in eighteen files would mean eighteen places for them to
   * drift, and the drift would be invisible: an icon drawn at a different stroke weight
   * looks like an icon from a different set, which is exactly what hand-writing them was
   * meant to avoid.
   *
   * What each icon file contributes is its geometry and nothing else.
   *
   * The construction is written in `docs/design/design-system.md` § Icons: a 24 box, a
   * 1.5 stroke, round caps, no fill, `currentColor` so that an icon takes the colour of
   * whatever it belongs to.
   */
  import type { Snippet } from 'svelte';

  interface Props {
    /**
     * What to call this icon out loud, when it is the only thing identifying a control.
     *
     * Absent is the common case and the right default: an icon beside its own word is
     * decoration, and announcing it twice is worse than not announcing it at all. Present
     * turns the drawing into an image with a name.
     */
    label?: string | undefined;
    /** The geometry: the paths, circles and rectangles this particular icon is made of. */
    children: Snippet;
  }

  const { label, children }: Props = $props();
</script>

<svg
  class="icon"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  role={label === undefined ? 'presentation' : 'img'}
  aria-hidden={label === undefined ? 'true' : undefined}
  aria-label={label}
>
  {#if label !== undefined}
    <title>{label}</title>
  {/if}
  {@render children()}
</svg>

<style>
  .icon {
    /* Sized in one place, so a row of icons and text always lines up. A caller that needs
     * a different size sets `--icon-size` on an ancestor rather than on the icon. */
    width: var(--icon-size);
    height: var(--icon-size);
    flex: none;
  }
</style>
