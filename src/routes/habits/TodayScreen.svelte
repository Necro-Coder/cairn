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
  import type { DayState, HabitFilter, HabitSummary } from '../../lib/ipc.types';
  import Badge from '../../lib/shell/Badge.svelte';
  import EmptyState from '../../lib/shell/EmptyState.svelte';
  import ScreenHeader from '../../lib/shell/ScreenHeader.svelte';
  import { sectionOf } from '../../lib/shell/sections';
  import AsyncView from '../../lib/ui/AsyncView.svelte';
  import { load, loading, messageFor, type Async } from '../../lib/ui/async';
  import {
    countsQuantity,
    forToday,
    isAtRisk,
    isMet,
    markText,
    move,
    orderOf,
    replace,
    streakText,
    todayText,
    without,
  } from './today';

  interface Props {
    /** How to open one habit, which is the only route into its own screen. */
    onOpen: (id: string) => void;
    /** How to start describing a habit that does not exist yet. */
    onCreate: () => void;
  }

  const { onOpen, onCreate }: Props = $props();

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

  /** Which list is being looked at: the ones being tracked, or the ones put away. */
  let filter = $state<HabitFilter>('active');

  /**
   * What just happened, said in one line above the list.
   *
   * A row disappearing and an order changing are both changes with nothing on screen to point
   * at afterwards, so they are said rather than left to be noticed. It is a live region for
   * that reason and no other, and it is emptied as soon as the next thing happens.
   */
  let announcement = $state('');

  /** Reads the whole list once. Called on mount, on a change of filter, and never in a loop. */
  async function read(): Promise<void> {
    const wanted = filter;
    view = await load(
      () => ipc.listHabits(wanted),
      // A list of archived habits shows every one of them: an archived habit is not asked
      // anything of today, so filtering it by today's schedule would empty the screen.
      (habits) => (wanted === 'active' ? forToday(habits).length === 0 : habits.length === 0),
    );
  }

  /** Switches list, which is a read of the other one and not a filter over what is here. */
  async function show(wanted: HabitFilter): Promise<void> {
    filter = wanted;
    announcement = '';
    view = loading();
    await read();
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

  /**
   * The rows to draw.
   *
   * In the list being tracked, that is every state but the one today never asked about. In
   * the list of things put away it is all of them: an archived habit is asked nothing today,
   * so filtering that list by today's schedule would empty the screen.
   */
  function rows(habits: readonly HabitSummary[]): readonly HabitSummary[] {
    return filter === 'active' ? forToday(habits) : habits;
  }

  /**
   * Puts a habit away, or takes it back out, and removes the row it was on.
   *
   * The row goes and the rest stay. Reading the whole list again over one row that is no
   * longer in it would redraw everything and lose the place somebody was in.
   *
   * Pressing twice does nothing the first press did not: the core is told what the habit
   * should now be rather than to toggle it, so a second press asks for the same state.
   */
  async function archive(habit: HabitSummary, away: boolean): Promise<void> {
    try {
      await ipc.archiveHabit(habit.id, away);
      if (view.status === 'ready') {
        const left = without(view.value, habit.id);
        view = left.length === 0 ? { status: 'empty' } : { status: 'ready', value: left };
      }
      announcement = away
        ? `${habit.name} se ha guardado. Está en «Archivados».`
        : `${habit.name} vuelve a la lista de hoy, con la racha que tenía.`;
    } catch (thrown: unknown) {
      announcement = describe(thrown);
      await read();
    }
  }

  /** Whatever the core refused, said in the one sentence this module has for it. */
  function describe(thrown: unknown): string {
    const error = thrown as { kind?: unknown };
    return typeof error.kind === 'string'
      ? messageFor(error as Parameters<typeof messageFor>[0])
      : 'No se ha podido completar.';
  }

  /**
   * Moves one habit a place and tells the core the whole new order.
   *
   * One call per press, and each one carries every habit there is. The core refuses a partial
   * order, and it is right to: two windows sending halves would interleave into an order
   * neither of them asked for.
   *
   * A refusal is not smoothed over. The list is read again and what was on screen is thrown
   * away, because the file is what the order actually is, and somebody is told in a sentence
   * rather than watching their rows quietly spring back.
   */
  async function reorder(habit: HabitSummary, by: -1 | 1): Promise<void> {
    if (view.status !== 'ready') {
      return;
    }
    const moved = move(view.value, habit.id, by);
    if (moved === view.value) {
      return;
    }
    const at = moved.findIndex((each) => each.id === habit.id);
    view = { status: 'ready', value: moved };
    announcement = `${habit.name} ahora es el ${String(at + 1)} de ${String(moved.length)}.`;
    try {
      await ipc.reorderHabits(orderOf(moved));
    } catch (thrown: unknown) {
      announcement = `${describe(thrown)} Se ha vuelto a leer la lista.`;
      await read();
    }
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
  lede={filter === 'active'
    ? 'Lo que hoy te pide, y nada más. Cada hábito guarda su año entero dentro.'
    : 'Lo que guardaste. Siguen teniendo toda su historia, y vuelven con la racha que tenían.'}
/>

<div class="filter" role="group" aria-label="Qué lista mirar">
  <button
    type="button"
    class:chosen={filter === 'active'}
    aria-pressed={filter === 'active'}
    onclick={() => void show('active')}>Hoy</button
  >
  <button
    type="button"
    class:chosen={filter === 'archived'}
    aria-pressed={filter === 'archived'}
    onclick={() => void show('archived')}>Archivados</button
  >
</div>

<!--
  A row disappearing and an order changing are both changes with nothing left on screen to
  point at afterwards. The design system rules out announcing form errors and the lock
  countdown; it does not rule out saying that something moved.
-->
<p class="said" aria-live="polite">{announcement}</p>

<AsyncView state={view} label="Leyendo tus hábitos…">
  {#snippet empty()}
    <EmptyState
      {section}
      sentence={filter === 'active'
        ? 'Hoy no hay nada que marcar. Cuando crees un hábito, aparecerá aquí el día que toque.'
        : 'No has guardado ningún hábito. Los que guardes aparecerán aquí, con su historia entera.'}
      action="Añadir hábito"
      onAction={onCreate}
    />
  {/snippet}

  {#snippet ready(habits: readonly HabitSummary[])}
    <ul class="rows">
      {#each rows(habits) as habit, at (habit.id)}
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

          {#if filter === 'active'}
            <!-- Two buttons rather than a drag, and never a drag alone. Every gesture in this
                 application has a keyboard equivalent, which is the promise the undecorated
                 window already cost us once and is not allowed to cost us twice. -->
            <button
              type="button"
              class="open"
              aria-label={`Subir ${habit.name}`}
              disabled={at === 0}
              onclick={() => void reorder(habit, -1)}>Subir</button
            >
            <button
              type="button"
              class="open"
              aria-label={`Bajar ${habit.name}`}
              disabled={at === rows(habits).length - 1}
              onclick={() => void reorder(habit, 1)}>Bajar</button
            >
            <button
              type="button"
              class="open"
              aria-label={`Guardar ${habit.name} en archivados`}
              onclick={() => void archive(habit, true)}>Archivar</button
            >
          {:else}
            <button
              type="button"
              class="open"
              aria-label={`Devolver ${habit.name} a la lista de hoy`}
              onclick={() => void archive(habit, false)}>Desarchivar</button
            >
          {/if}

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

    <!-- After the list rather than above it: the reason somebody opened this screen is the
         list, and the way to add one more is what they look for when they are done with it. -->
    <p class="add">
      <button
        type="button"
        onclick={() => {
          onCreate();
        }}>Añadir hábito</button
      >
    </p>
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

  /* Two lists, one of which is being looked at. A pressed button rather than a tab: the
   * strip at the top of the window is for the five sections, and this is not one. */
  .filter {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-6);
  }

  .filter button {
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font: inherit;
    font-size: var(--text-sm);
  }

  .filter .chosen {
    border-color: var(--colour-text);
    background-color: var(--colour-surface-sunken);
    font-weight: var(--weight-semibold);
  }

  /* Empty most of the time, and it keeps no height when it is: a line of nothing above a
   * list would be a gap nobody could account for. */
  .said:empty {
    display: none;
  }

  .said {
    max-width: var(--measure);
    margin: var(--space-4) 0 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .add {
    margin: var(--space-5) 0 0;
  }

  .add button {
    padding: var(--space-3) var(--space-4);
    border: 0;
    border-radius: var(--radius-sm);
    background-color: var(--colour-accent);
    color: var(--colour-accent-contrast);
    font: inherit;
    font-weight: var(--weight-semibold);
    transition: background-color var(--duration-fast) var(--easing);
  }

  .add button:hover {
    background-color: var(--colour-accent-strong);
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
