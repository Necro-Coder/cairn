<script lang="ts">
  /**
   * A year of one habit, at a glance.
   *
   * Given a `Heatmap` and nothing else. It asks the core for nothing, decides nothing about
   * any day, and draws the same grid every time for the same answer. Which year is on screen
   * and where it came from belong to whatever mounts it.
   *
   * The grid itself is hidden from assistive technology and the sentence under it carries the
   * meaning. Three hundred and sixty-six cells announced one by one say less than one line of
   * Spanish, and the design system settles that trade once for the whole application rather
   * than leaving it to each grid.
   */
  import type { Heatmap as HeatmapAnswer } from '../ipc.types';

  import { place, treatmentOf } from './heatmap';

  interface Props {
    /** The year to draw, exactly as the core answered it. */
    heatmap: HeatmapAnswer;
    /** One sentence saying what the grid is, which is what a screen reader is given. */
    caption: string;
  }

  const { heatmap, caption }: Props = $props();

  const grid = $derived(place(heatmap));
</script>

<figure class="year">
  <!-- The grid is decoration as far as assistive technology is concerned: the caption is
       the content. A cell-by-cell reading of a year is noise, not a description. -->
  <div class="grid" aria-hidden="true">
    {#each grid.cells as cell, at (at)}
      {#if cell === null}
        <!-- Not a day of this year. Drawn as nothing at all, which is not «no data». -->
        <span class="gap"></span>
      {:else}
        <span class="cell {treatmentOf(cell.state)}"></span>
      {/if}
    {/each}
  </div>

  <figcaption>{caption}</figcaption>
</figure>

<style>
  /*
   * The figure is what scrolls, not the page. Below the width where a square stops being a
   * square the year goes sideways rather than squashing.
   */
  .year {
    margin: 0;
    overflow-x: auto;
  }

  /*
   * Fifty-three columns always, and fluid ones, so the year narrows with the window instead
   * of overflowing it. Fifty-three rather than fifty-two because a year crosses that many
   * calendar weeks whenever it does not start on a Monday, which is six years out of seven,
   * and a grid that lost a column in those years would change shape without warning. A year
   * that reaches only fifty-two leaves the last column empty, which is one column of paper
   * and not a shape anybody reads.
   */
  .grid {
    display: grid;
    grid-template-columns: repeat(53, 1fr);
    grid-template-rows: repeat(7, auto);
    grid-auto-flow: column;
    gap: var(--space-1);
    min-width: var(--heatmap-min-width);
  }

  .cell,
  .gap {
    aspect-ratio: 1;
  }

  /* A square of the grid that is not a day of this year. Nothing is drawn. */
  .gap {
    visibility: hidden;
  }

  .cell {
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
  }

  /* Filled. The plainest square there is, because it is the one somebody looks for. */
  .done {
    background-color: var(--colour-heatmap-done);
  }

  /*
   * Filled, with a hole. A day that went beyond what was asked is still a day that was kept,
   * so it keeps the same fill and gains a mark, rather than becoming a sixth colour nobody
   * would be able to place.
   */
  .extra {
    background-color: var(--colour-heatmap-done);
  }

  .extra::after {
    width: var(--heatmap-dot-size);
    height: var(--heatmap-dot-size);
    border-radius: var(--radius-sm);
    background-color: var(--colour-heatmap-extra);
    content: '';
  }

  /* Outlined and empty. The shape says the day was asked for and the inside says it was not
   * answered, which is the difference a colour alone would not carry in greyscale. */
  .missed {
    border: var(--border-width) solid var(--colour-heatmap-missed);
  }

  /* A dot on its own, no square: the habit never asked about this day. */
  .not-scheduled::after {
    width: var(--heatmap-dot-size);
    height: var(--heatmap-dot-size);
    border-radius: var(--radius-sm);
    background-color: var(--colour-heatmap-quiet);
    content: '';
  }

  /* Dashed. Before the habit existed, or a day that has not happened yet. */
  .no-data {
    border: var(--border-width) dashed var(--colour-heatmap-quiet);
  }

  figcaption {
    max-width: var(--measure);
    margin-top: var(--space-4);
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }
</style>
