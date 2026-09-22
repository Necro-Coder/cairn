<script lang="ts">
  /**
   * What today asks for, and the one gesture that answers it.
   *
   * The screen that gets opened most, so it does the least: one call for the whole list, one
   * row per habit today actually asks something of, and a control that marks it. The year,
   * the detail and the form are other screens.
   *
   * Everything it draws was decided in Rust. The run, the square, the week in progress and
   * what counts as met all arrive worked out; this file chooses words and edges for them.
   * Nothing here counts a day.
   *
   * Marking replaces one row rather than asking for the list again. The core answers the
   * press with the square it decided, which is what the row draws at once, and the habit is
   * then read back for the run, which the square alone cannot say. Two quick presses on the
   * same habit are the reason for the sequence number: the row ends up showing the answer to
   * the last press, never the answer that happened to arrive last.
   */
  import { ipc } from '$ipc';
  import type { DayState, HabitSummary } from '../../lib/ipc.types';
  import Badge from '../../lib/shell/Badge.svelte';
  import EmptyState from '../../lib/shell/EmptyState.svelte';
  import ScreenHeader from '../../lib/shell/ScreenHeader.svelte';
  import { sectionOf } from '../../lib/shell/sections';
  import AsyncView from '../../lib/ui/AsyncView.svelte';
  import { load, loading, type Async } from '../../lib/ui/async';
  import {
    countsQuantity,
    forToday,
    isAtRisk,
    isMet,
    markText,
    replace,
    streakText,
    todayText,
  } from './today';

  interface Props {
    /** How to open one habit, which is the only route into its own screen. */
    onOpen: (id: string) => void;
  }

  const { onOpen }: Props = $props();

  const section = sectionOf('habits');

  let view = $state<Async<readonly HabitSummary[]>>(loading());

  /**
   * Which habit has its amount field open, and what is typed in it.
   *
   * One at a time, because the field takes the focus when it opens and two of them would be
   * two places the focus could have gone.
   */
  let editing = $state<string | null>(null);
  let typed = $state('');

  /**
   * How many presses each habit has had, so a slow answer to an early press is discarded.
   *
   * A plain object and not a reactive map on purpose: nothing on screen is drawn from it. It
   * is bookkeeping about calls in flight, and making it reactive would redraw every row each
   * time somebody pressed one.
   *
   * Without it, pressing twice quickly leaves the row showing whichever reply the network
   * happened to deliver second, which is a row nobody asked for.
   */
  const presses: Record<string, number> = {};

  /** Reads the whole list once. Called on mount and never in a loop. */
  async function read(): Promise<void> {
    view = await load(
      () => ipc.listHabits('active'),
      (habits) => forToday(habits).length === 0,
    );
  }

  $effect(() => {
    void read();
  });

  /**
   * Reads it again when the window comes back, which is what keeps midnight honest.
   *
   * The day each row carries is the day the core judged it against, and a window left open
   * across midnight is holding yesterday. The core would take it — yesterday is inside the
   * window that may be marked — so nothing would refuse, and somebody would mark the wrong
   * day while the row said today. Coming back to the window is the moment to ask again.
   */
  $effect(() => {
    const again = (): void => {
      void read();
    };
    window.addEventListener('focus', again);
    return () => {
      window.removeEventListener('focus', again);
    };
  });

  /** The rows today asks something of, which is every state but the one it never asked about. */
  function rows(habits: readonly HabitSummary[]): readonly HabitSummary[] {
    return forToday(habits);
  }

  /**
   * Puts the core's verdict about one day onto the row it belongs to.
   *
   * Only the square. The run is what the read that follows brings, because a square cannot
   * say whether the day before it was kept.
   */
  function applyDay(id: string, today: DayState): void {
    if (view.status !== 'ready') {
      return;
    }
    const current = view.value.find((habit) => habit.id === id);
    if (current !== undefined) {
      view = { status: 'ready', value: replace(view.value, { ...current, today }) };
    }
  }

  /**
   * Marks or unmarks one habit's today, and redraws that row alone.
   *
   * `null` as the amount is what a habit that is simply done or not sends, and the core reads
   * it as the switch it is. A habit that counts a quantity always sends a number.
   */
  async function mark(habit: HabitSummary, amount: number | null): Promise<void> {
    const press = (presses[habit.id] ?? 0) + 1;
    presses[habit.id] = press;
    try {
      // The day the row itself carries, handed straight back. The core is the one that knows
      // which day today is here — it holds the zone and the offset this person's day starts
      // at — and it checks the number again on the way in, so this side never guesses one.
      const today = await ipc.toggleHabitDay(habit.id, habit.todayDay, amount);
      if (presses[habit.id] !== press) {
        return;
      }
      applyDay(habit.id, today);
      const fresh = await ipc.getHabit(habit.id);
      if (presses[habit.id] === press && view.status === 'ready') {
        view = { status: 'ready', value: replace(view.value, fresh) };
      }
    } catch {
      // The list is what the core has; asking again is the only honest way back from a press
      // that did not land, and it is cheaper than guessing what the row should now say.
      await read();
    }
  }

  /** Opens the amount field on a habit that counts one, with what it has today already in it. */
  function beginAmount(habit: HabitSummary): void {
    editing = habit.id;
    typed = String(habit.today.state === 'noData' ? 0 : habit.today.amount);
  }

  /** Sends what was typed, if it is a number, and closes the field either way. */
  async function commitAmount(habit: HabitSummary): Promise<void> {
    const amount = Number.parseInt(typed, 10);
    editing = null;
    if (Number.isNaN(amount)) {
      return;
    }
    await mark(habit, amount);
  }

  /** What the control does: a field for a quantity, a switch for anything else. */
  async function press(habit: HabitSummary): Promise<void> {
    if (countsQuantity(habit)) {
      beginAmount(habit);
      return;
    }
    await mark(habit, null);
  }
</script>

<ScreenHeader
  {section}
  title="Hábitos"
  lede="Lo que hoy te pide, y nada más. El año, el detalle y los archivados están dentro de cada hábito."
/>

<AsyncView state={view} label="Leyendo tus hábitos…">
  {#snippet empty()}
    <EmptyState
      {section}
      sentence="Hoy no hay nada que marcar. Cuando crees un hábito, aparecerá aquí el día que toque."
      action="Añadir hábito"
      note="El formulario llega en el siguiente paso de esta fase. La lista y el marcado ya funcionan."
    />
  {/snippet}

  {#snippet ready(habits: readonly HabitSummary[])}
    <ul class="rows">
      {#each rows(habits) as habit (habit.id)}
        <li class="row" class:met={isMet(habit.today)}>
          <button
            type="button"
            class="mark"
            title={markText(habit)}
            aria-label={markText(habit)}
            onclick={() => void press(habit)}
          >
            <span class="box" aria-hidden="true"></span>
            <span class="words">
              <!-- Always text. It is what the person typed, and one day it will arrive
                   through an import written by somebody else. -->
              <span class="name">{habit.name}</span>
              <span class="today">{todayText(habit)}</span>
            </span>
            <span class="streak">{streakText(habit)}</span>
          </button>

          {#if isAtRisk(habit)}
            <Badge tone={section.tone} text="En riesgo" />
          {/if}

          <!-- A second control rather than the whole row, so the space bar keeps meaning
               «marca esto». A row that opened on the same key would make marking the one
               gesture somebody cannot do without looking. -->
          <button
            type="button"
            class="open"
            aria-label={`Abrir ${habit.name}`}
            onclick={() => {
              onOpen(habit.id);
            }}
          >
            Ver
          </button>

          {#if editing === habit.id}
            <form class="amount" onsubmit={() => void commitAmount(habit)}>
              <label for={`amount-${habit.id}`}>
                Cuánto llevas hoy{habit.unit === null ? '' : ` (${habit.unit})`}
              </label>
              <input
                id={`amount-${habit.id}`}
                type="number"
                inputmode="numeric"
                min="0"
                bind:value={typed}
                onkeydown={(event) => {
                  if (event.key === 'Escape') {
                    editing = null;
                  }
                }}
              />
              <button type="submit">Anotar</button>
            </form>
          {/if}
        </li>
      {/each}
    </ul>
  {/snippet}
</AsyncView>

<style>
  .rows {
    margin: var(--space-6) 0 0;
    padding: 0;
    list-style: none;
  }

  /* Separated by a rule, never by alternating background: the design system is explicit. */
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) 0;
    border-bottom: var(--border-width) solid var(--colour-border);
  }

  .mark {
    display: flex;
    flex: 1 1 var(--habit-row-min);
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-2) var(--space-3);
    border: 0;
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: inherit;
    text-align: left;
    transition: background-color var(--duration-fast) var(--easing);
  }

  /* Raises the surface rather than tinting it. */
  .mark:hover {
    background-color: var(--colour-surface-raised);
  }

  /*
   * The square, which is the whole of what says whether today is answered. Filled when it
   * is, hollow when it is not, and it is the edge rather than the fill that carries it for
   * anybody who cannot tell the two colours apart.
   */
  .box {
    flex: none;
    width: var(--habit-box-size);
    height: var(--habit-box-size);
    border: var(--rule-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
  }

  .met .box {
    border-color: var(--colour-positive);
    background-color: var(--colour-positive);
  }

  .words {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    gap: var(--space-1);
  }

  .name {
    font-weight: var(--weight-semibold);
  }

  .today {
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .streak {
    flex: none;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }

  /* Bordered, not accented: there is one accent per screen and it is not this. */
  .open {
    flex: none;
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font: inherit;
    font-size: var(--text-sm);
  }

  /* In place, under the row it belongs to. Nothing on this screen opens over anything. */
  .amount {
    display: flex;
    flex: 1 0 100%;
    flex-wrap: wrap;
    align-items: end;
    gap: var(--space-3);
    padding: var(--space-3) 0 var(--space-3) var(--space-5);
  }

  .amount label {
    flex: 1 1 100%;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .amount input {
    width: var(--habit-amount-width);
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: var(--colour-surface-sunken);
    color: var(--colour-text);
    font: inherit;
  }

  .amount button {
    padding: var(--space-2) var(--space-4);
    border: 0;
    border-radius: var(--radius-sm);
    background-color: var(--colour-accent);
    color: var(--colour-accent-contrast);
    font-weight: var(--weight-semibold);
  }
</style>
