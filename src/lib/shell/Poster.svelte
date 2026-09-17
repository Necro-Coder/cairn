<script lang="ts">
  /**
   * The full composition, which exists on exactly one screen: the lock screen.
   *
   * Four shapes, in a fixed arrangement, exactly as the design system describes them — a
   * large vermilion circle breaking the top right corner, a blue half circle sitting under
   * it, an ink rule crossing the width, and one small yellow dot low on the opposite side.
   * Fixed rather than varied, because this screen is seen twice a day and a composition that
   * moves is a composition that irritates.
   *
   * The four prohibitions hold here as everywhere: nothing is on top of a mark and a mark is
   * on top of nothing, it never animates, it takes no pointer events, and it means nothing —
   * it is `aria-hidden`, and it disappears in forced-colors mode. The shapes are sized as
   * proportions of the region rather than in pixels, so the arrangement is the same shape at
   * every window size until it is removed altogether.
   */
</script>

<div class="poster" aria-hidden="true">
  <span class="circle mark"></span>
  <span class="half mark"></span>
  <span class="rule mark"></span>
  <span class="dot mark"></span>
</div>

<style>
  .poster {
    position: relative;
    /* Square, so the arrangement keeps its proportions rather than stretching with the
     * column it sits in. */
    aspect-ratio: 1;
    width: 100%;
    /* Nothing overflows: a shape cut off by an edge that is not the window's reads as a
     * rendering fault rather than as a composition. */
  }

  .poster span {
    position: absolute;
  }

  .circle {
    top: 0;
    right: 0;
    width: 58%;
    aspect-ratio: 1;
    border-radius: 50%;
    background-color: var(--mark-vermilion);
  }

  /* Sitting on the circle's lower edge: touching it, never over it, because two marks that
   * overlap produce a third colour. */
  .half {
    top: 58%;
    right: 14%;
    width: 26%;
    aspect-ratio: 2;
    /* Rounded along the top edge only, flat along the bottom. */
    border-radius: 100% 100% 0 0;
    background-color: var(--mark-blue);
  }

  .rule {
    top: 76%;
    left: 0;
    width: 100%;
    height: var(--rule-width);
    background-color: var(--mark-ink);
  }

  .dot {
    top: 86%;
    left: 8%;
    width: 6%;
    aspect-ratio: 1;
    border-radius: 50%;
    background-color: var(--mark-yellow);
  }

  /* Below the window minimum the composition drops to one shape rather than shrinking into
   * the form beside it, and below the phone shape it goes altogether. */
  @media (max-width: 880px) {
    .half,
    .rule,
    .dot {
      display: none;
    }
  }

  @media (max-width: 420px) {
    .poster {
      display: none;
    }
  }
</style>
