<script lang="ts">
  /**
   * The brand glyph: a cairn of three stones, flat, in full vermilion.
   *
   * Not an icon, and not in the count of eighteen. An icon marks a control; this marks the
   * application, and it is the one thing in the interface allowed to be vermilion without
   * being an action.
   *
   * That exception is written down in `docs/design/design-system.md` § Colour, and it is
   * fenced by two prohibitions that this component cannot enforce on its own and that
   * every caller has to respect:
   *
   * 1. It never appears next to a primary button. Two vermilion shapes in one field of
   *    view and the rule that vermilion means action is gone.
   * 2. It is never clickable, and never inside anything clickable.
   *
   * Always hidden from assistive technology, with no way to give it a name: everywhere it
   * appears, the word "Cairn" is written beside it, and announcing both would say the name
   * of the application twice.
   *
   * Filled rather than stroked, which is why it is not built on `IconFrame`: the stones
   * are flat shapes in the manner of the references, not a line drawing.
   */

  interface Props {
    /**
     * Whether this is the seal of the lock screen rather than the corner of the title bar.
     *
     * A variant and not a size, because a size would have to arrive in a `style` attribute
     * and the content security policy drops those. There are two places and two sizes.
     */
    seal?: boolean;
  }

  const { seal = false }: Props = $props();
</script>

<svg
  class="brand-mark mark"
  viewBox="0 0 24 24"
  fill="var(--brand-glyph)"
  role="presentation"
  aria-hidden="true"
  class:seal
>
  <!-- Base: the widest and heaviest stone, seated low. -->
  <rect x="3" y="17" width="18" height="4" />
  <!-- The middle stone, narrower, with a clear gap under it. -->
  <rect x="5.5" y="11" width="13" height="4.5" />
  <!-- The capstone: round, which is what makes the pile read as a marker. -->
  <circle cx="12" cy="5.75" r="4" />
</svg>

<style>
  .brand-mark {
    width: var(--icon-size);
    height: var(--icon-size);
    flex: none;
  }

  /* The lock screen, where it sits beside the product name at display size. */
  .seal {
    width: var(--text-2xl);
    height: var(--text-2xl);
  }
</style>
