<script lang="ts">
  /**
   * One habit, and everything it has ever done.
   *
   * Three calls when it opens — the habit, the year, and the numbers — and one more each time
   * somebody steps to another year. Never one per day and never one per square: a year is a
   * single answer from the core, and the arrows replace that answer without asking for the
   * habit or its numbers again, because neither of those changes when the year on screen does.
   *
   * Nothing here counts anything. The run, the record and the month's percentage arrive worked
   * out, percentage included, so that a second rounding rule on this side cannot disagree with
   * the first about the same two numbers.
   *
   * This is the only screen the note appears on. It is the one sealed column of this module and
   * it crosses the boundary here alone, which is why the list does not carry it.
   */
  import { ipc } from '$ipc';
  import type { HabitDetail, HabitStats, Heatmap as HeatmapAnswer } from '../../lib/ipc.types';
  import Badge from '../../lib/shell/Badge.svelte';
  import ScreenHeader from '../../lib/shell/ScreenHeader.svelte';
  import { sectionOf } from '../../lib/shell/sections';
  import AsyncView from '../../lib/ui/AsyncView.svelte';
  import Heatmap from '../../lib/ui/Heatmap.svelte';
  import { load, loading, type Async } from '../../lib/ui/async';
  import { canGoBack, canGoForward, daysText, ratioText, weekText, yearOf } from './detail';
  import { isAtRisk, streakText, todayText } from './today';

  interface Props {
    /** Which habit to open. */
    id: string;
    /** How to get back to the list, which is where every route into this screen came from. */
    onBack: () => void;
    /** How to open the form on this habit. */
    onEdit: () => void;
  }

  const { id, onBack, onEdit }: Props = $props();

  const section = sectionOf('habits');

  /** The habit and its numbers: read once when the screen opens, and not again per year. */
  interface Opened {
    habit: HabitDetail;
    stats: HabitStats;
  }

  let opened = $state<Async<Opened>>(loading());

  /** The year on screen, replaced on its own each time an arrow is pressed. */
  let year = $state<Async<HeatmapAnswer>>(loading());

  /**
   * Which year is on screen, kept beside the answer so an arrow knows where it is going.
   *
   * Taken from the answer rather than from a clock: `heatmap.year` is the year the core drew,
   * and stepping from anything else would eventually step from a year nobody is looking at.
   */
  let showing = $state(0);

  /** Reads the habit and its numbers. Called when the screen opens, and when it is repaired. */
  async function readHabit(): Promise<void> {
    opened = await load(
      async () => {
        const [habit, stats] = await Promise.all([ipc.getHabit(id), ipc.habitStats(id)]);
        return { habit, stats };
      },
      // Never empty. A habit that exists has a screen, however little is on it.
      () => false,
    );
  }

  /** Reads one year. The only call an arrow makes. */
  async function readYear(wanted: number): Promise<void> {
    showing = wanted;
    year = await load(
      () => ipc.habitHeatmap(id, wanted),
      () => false,
    );
  }

  $effect(() => {
    void (async () => {
      await readHabit();
      if (opened.status === 'ready') {
        // The core's own answer to what year it is, carried on the square it judged today
        // against. A `Date` here would be a second opinion about where the year ends.
        await readYear(yearOf(opened.value.habit.todayDay));
      }
    })();
  });

  /** This year, as the core counts years. Everything about the arrows is bounded by it. */
  const thisYear = $derived(
    opened.status === 'ready' ? yearOf(opened.value.habit.todayDay) : showing,
  );
</script>

<AsyncView state={opened} label="Leyendo el hábito…">
  {#snippet empty()}
    <p>Ese hábito ya no está.</p>
  {/snippet}

  {#snippet ready(detail: Opened)}
    <ScreenHeader
      {section}
      title={detail.habit.name}
      lede={detail.habit.archived ? 'Guardado. No aparece en la lista de hoy.' : undefined}
    />

    <p class="back">
      <button type="button" class="plain" onclick={onBack}>Volver a la lista de hoy</button>
    </p>

    <section class="numbers">
      <h2>Los números</h2>

      <dl>
        <div class="figure">
          <dt>Racha actual</dt>
          <dd>
            {daysText(detail.stats.current.days)}
            {#if isAtRisk({ ...detail.habit, streak: detail.stats.current })}
              <Badge tone={section.tone} text="En riesgo" />
            {/if}
          </dd>
        </div>

        <div class="figure">
          <dt>La más larga</dt>
          <dd>{daysText(detail.stats.longest)}</dd>
        </div>

        <div class="figure">
          <dt>Este mes</dt>
          <!-- Both numbers, never the percentage alone: «90%» of an unknown number of days
               is a figure nobody can act on. -->
          <dd>{ratioText(detail.stats.monthCompletion)}</dd>
        </div>

        <div class="figure">
          <dt>Hoy</dt>
          <dd>{todayText(detail.habit)}</dd>
        </div>
      </dl>

      {#if weekText(detail.stats.current) !== null}
        <p class="week">{weekText(detail.stats.current)}</p>
      {/if}

      <p class="run">{streakText({ ...detail.habit, streak: detail.stats.current })}</p>
    </section>

    <section class="calendar">
      <div class="years">
        <h2>El año {showing}</h2>

        <div class="arrows">
          <button
            type="button"
            disabled={year.status !== 'ready' || !canGoBack(showing, year.value.firstYearWithData)}
            title={year.status === 'ready' && canGoBack(showing, year.value.firstYearWithData)
              ? `Ir a ${showing - 1}`
              : 'No hay nada marcado antes de este año'}
            onclick={() => void readYear(showing - 1)}
          >
            {showing - 1}
          </button>

          <button
            type="button"
            disabled={!canGoForward(showing, thisYear)}
            title={canGoForward(showing, thisYear)
              ? `Ir a ${showing + 1}`
              : 'Ese año todavía no ha empezado'}
            onclick={() => void readYear(showing + 1)}
          >
            {showing + 1}
          </button>
        </div>
      </div>

      <AsyncView state={year} label="Leyendo el año…">
        {#snippet empty()}
          <p>No hay nada que dibujar de ese año.</p>
        {/snippet}

        {#snippet ready(drawn: HeatmapAnswer)}
          <!--
            The one live region in this application, and it is here because stepping a year
            changes the whole picture with nothing else on screen moving. The design system
            rules out announcing form errors and the lock countdown; it does not rule out
            saying which year somebody just arrived at.
          -->
          <div aria-live="polite">
            <Heatmap
              heatmap={drawn}
              caption={`Un cuadro por cada día de ${String(drawn.year)}. Relleno, cumplido; con un hueco, por encima de lo pedido; con borde, fallado; un punto, día que no tocaba; a rayas, fuera de la vida del hábito.`}
            />
          </div>
        {/snippet}
      </AsyncView>
    </section>

    {#if detail.habit.notes !== null}
      <section class="notes">
        <h2>Tus notas</h2>
        <!-- Always text. It is what the person wrote, and a note is the one column of this
             module that is sealed in the database. -->
        <p>{detail.habit.notes}</p>
      </section>
    {/if}

    <section class="actions">
      <h2>Este hábito</h2>

      <div class="buttons">
        <button type="button" class="primary" onclick={onEdit}>Editar</button>
      </div>

      <p class="pending">Archivar y borrar llegan en el siguiente paso de esta fase.</p>
      <Badge tone={section.tone} text="En desarrollo" />
    </section>
  {/snippet}
</AsyncView>

<style>
  section {
    margin-top: var(--space-7);
  }

  h2 {
    margin: 0 0 var(--space-4);
    font-size: var(--text-lg);
    letter-spacing: normal;
  }

  .back {
    margin: var(--space-5) 0 0;
  }

  .plain {
    padding: 0;
    border: 0;
    background: none;
    color: var(--colour-accent);
    font: inherit;
    text-decoration: underline;
  }

  dl {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-6);
    margin: 0;
  }

  .figure {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  dt {
    color: var(--colour-text-muted);
    font-size: var(--text-label);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
  }

  dd {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--text-xl);
    font-variant-numeric: tabular-nums;
  }

  .week,
  .run,
  .pending {
    max-width: var(--measure);
    margin: var(--space-4) 0 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .years {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .arrows {
    display: flex;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
  }

  .arrows button {
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font: inherit;
    font-variant-numeric: tabular-nums;
  }

  /* A dead control explains itself rather than going quiet: its title says why. */
  .arrows button:disabled {
    border-color: var(--colour-border);
    color: var(--colour-text-faint);
  }

  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }

  /* One accent per screen, and on this one it is the way into the form. */
  .buttons .primary {
    padding: var(--space-3) var(--space-4);
    border: 0;
    border-radius: var(--radius-sm);
    background-color: var(--colour-accent);
    color: var(--colour-accent-contrast);
    font: inherit;
    font-weight: var(--weight-semibold);
    transition: background-color var(--duration-fast) var(--easing);
  }

  .buttons .primary:hover {
    background-color: var(--colour-accent-strong);
  }

  .notes p {
    max-width: var(--measure);
    margin: 0;
    white-space: pre-wrap;
  }
</style>
