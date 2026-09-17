<script lang="ts">
  /**
   * A small geometric composition: a vermilion circle, a blue half circle, a yellow dot.
   *
   * The part of the poster references that gives the application a face, and the part that
   * can ruin it, so where it is allowed is a closed list in
   * `docs/design/design-system.md` § Geometric marks. This component enforces the four
   * prohibitions that can be enforced in code: it takes no pointer events, it is hidden
   * from assistive technology, it is removed in forced-colors mode, and it never animates.
   *
   * It lives in its own region of the layout with nothing on top of it. Where the window is
   * too narrow for that region the shapes are removed rather than shrunk into the content:
   * below 880px only the circle is left, and below 420px none of them.
   */

  interface Props {
    /**
     * How many shapes are drawn.
     *
     * Three for an empty state and the panel header, two for creating the vault and the
     * damaged-header screen. Those are the budgets in the design system's table, and they
     * are a number here rather than a note in a review comment because the difference
     * between a face and a decorated page is one shape too many.
     */
    shapes?: 2 | 3;
  }

  const { shapes = 3 }: Props = $props();
</script>

<div class="marks" aria-hidden="true">
  <span class="circle mark"></span>
  <span class="half mark"></span>
  {#if shapes > 2}
    <span class="dot mark"></span>
  {/if}
</div>

<style>
  .marks {
    display: flex;
    flex: none;
    align-items: flex-end;
    gap: var(--space-3);
  }

  /* The three are proportions of the whole composition rather than three sizes of their
   * own, so one token decides how large a composition is and the arrangement never
   * changes shape between the places it is allowed. */
  .circle {
    width: calc(var(--empty-mark-max) * 0.5);
    aspect-ratio: 1;
    border-radius: 50%;
    background-color: var(--mark-vermilion);
  }

  .half {
    width: calc(var(--empty-mark-max) * 0.36);
    aspect-ratio: 2;
    /* A half circle: rounded along the top edge only, flat along the bottom. */
    border-radius: 100% 100% 0 0;
    background-color: var(--mark-blue);
  }

  .dot {
    width: calc(var(--empty-mark-max) * 0.11);
    aspect-ratio: 1;
    border-radius: 50%;
    background-color: var(--mark-yellow);
  }

  @media (max-width: 880px) {
    .half,
    .dot {
      display: none;
    }
  }

  @media (max-width: 420px) {
    .marks {
      display: none;
    }
  }
</style>
