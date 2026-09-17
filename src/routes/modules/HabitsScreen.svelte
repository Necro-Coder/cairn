<script lang="ts">
  /**
   * Habits, before there are any.
   *
   * Two blocks: the year, and the list. The year is drawn empty rather than left out
   * because it is the shape of this module — a habit is a thing you either did or did not
   * do on a day, and a grid of days is the only way that reads at a glance. Drawing it with
   * nothing in it is drawing the real screen of somebody's first day.
   *
   * There is no colour scale yet, and that is deliberate rather than pending. A scale
   * invented against no data would be a scale designed twice, and the second time would be
   * with real densities in front of us.
   */
  import Badge from '../../lib/shell/Badge.svelte';
  import EmptyState from '../../lib/shell/EmptyState.svelte';
  import ScreenHeader from '../../lib/shell/ScreenHeader.svelte';
  import { sectionOf } from '../../lib/shell/sections';

  const section = sectionOf('habits');

  /**
   * The grid is fifty-three weeks by seven days.
   *
   * Fifty-three rather than fifty-two because a year crosses that many calendar weeks
   * whenever it does not start on a Monday, which is six years out of seven. A grid that
   * lost a column in those years would be a grid that changed shape without warning.
   */
  const WEEKS = 53;
  const DAYS = WEEKS * 7;

  /** One entry per square. The index is its key and its only content. */
  const SQUARES = Array.from({ length: DAYS }, (_each, index) => index);
</script>

<ScreenHeader
  {section}
  title="Hábitos"
  lede="Lo que quieres hacer a diario, y si lo hiciste. Nada se registra todavía: esto es la forma que va a tener."
/>

<section class="year">
  <h2>Tu año</h2>

  <!-- Hidden from assistive technology on purpose: three hundred and seventy-one empty
       squares announced one by one say nothing that the sentence under them does not say
       better. It becomes a described image the day it carries data. -->
  <div class="grid" aria-hidden="true">
    {#each SQUARES as square (square)}
      <span class="square"></span>
    {/each}
  </div>

  <div class="foot">
    <p>Un cuadro por día del último año. Se irán marcando solos según registres hábitos.</p>
    <Badge tone={section.tone} text="En desarrollo" />
  </div>
</section>

<section class="list">
  <h2>Tus hábitos</h2>

  <EmptyState
    {section}
    sentence="Todavía no tienes ningún hábito."
    action="Añadir hábito"
    note="Crear y marcar hábitos llega en la fase de este módulo. Hasta entonces la pantalla enseña su forma, no sus datos."
  />
</section>

<style>
  section {
    margin-top: var(--space-7);
  }

  h2 {
    margin: 0 0 var(--space-4);
    font-size: var(--text-lg);
    letter-spacing: normal;
  }

  /*
   * Fifty-three fluid columns rather than a fixed square size, so the grid narrows with the
   * window instead of overflowing it. A scrolling strip here would be a scrollable region
   * with nothing to focus, which is the exact defect the accessibility gate caught in the
   * command palette.
   */
  .grid {
    display: grid;
    grid-template-columns: repeat(53, 1fr);
    grid-template-rows: repeat(7, auto);
    grid-auto-flow: column;
    gap: var(--space-1);
  }

  .square {
    aspect-ratio: 1;
    background-color: var(--colour-surface-sunken);
  }

  .foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-4);
  }

  .foot p {
    max-width: var(--measure);
    margin: 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .list {
    border-top: var(--border-width) solid var(--colour-border);
  }

  .list h2 {
    margin-top: var(--space-6);
  }
</style>
