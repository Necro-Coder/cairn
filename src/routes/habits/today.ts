/**
 * What today's list says about one habit, worked out from what the core already decided.
 *
 * Every function here reads a value the core sent and turns it into something to draw. Not
 * one of them counts a streak, judges a day or works out a percentage: those belong to the
 * domain in Rust, which is the only place they are tested, and a second opinion on this side
 * is a second opinion free to disagree about the same calendar.
 *
 * They live outside the component because a component cannot be run by `node --test`. What
 * is left in the screen is what genuinely needs a window.
 */

import type { DayState, HabitSummary } from '../../lib/ipc.types';

/**
 * Whether a habit belongs in today's list at all.
 *
 * A day the habit never asked about is not a day it is behind on. Drawing it would make
 * Sunday look like four failures and would teach somebody to stop reading the screen.
 *
 * The four other states all belong: `noData` is a habit that has not started yet, and
 * seeing it waiting is the point of having put a start date on it.
 */
export function isForToday(habit: HabitSummary): boolean {
  return habit.today.state !== 'notScheduled';
}

/**
 * Whether the run is alive and today is still open with nothing on it.
 *
 * Read from what the core sent, never worked out again. The core knows what today is in the
 * device's own zone and what the habit's schedule says; this side knows neither.
 */
export function isAtRisk(habit: HabitSummary): boolean {
  return habit.streak.atRisk;
}

/** Whether today's square counts as met, whichever way the habit is read. */
export function isMet(state: DayState): boolean {
  return state.state === 'done' || state.state === 'extra';
}

/**
 * Whether a habit counts a quantity, which is what naming a unit makes it do.
 *
 * The unit decides and not the target, because a weekly habit with no unit has a target
 * too: there it means how many days of the week, not how much of anything on one of them.
 */
export function countsQuantity(habit: HabitSummary): boolean {
  return habit.unit !== null;
}

/** Whether a habit is one to be cut down rather than built up. */
export function isNegative(habit: HabitSummary): boolean {
  return habit.direction === 'atMost';
}

/** How much was done today, which is zero on a day with no mark on it. */
export function amountToday(state: DayState): number {
  return state.state === 'noData' ? 0 : state.amount;
}

/** What today asked for, or nothing on a day that asked for nothing. */
export function targetToday(state: DayState): number | null {
  return state.state === 'noData' || state.state === 'notScheduled' ? null : state.target;
}

/**
 * What the row says about today, in the person's own terms.
 *
 * This is where a habit to be cut down reads backwards, and it has to read backwards
 * without anybody looking up what the arrow means: the good day of «sin azúcar» is the day
 * nobody touched, so the row says it is clean rather than saying it is done.
 */
export function todayText(habit: HabitSummary): string {
  if (countsQuantity(habit)) {
    const target = targetToday(habit.today);
    const unit = habit.unit ?? '';
    const amount = `${String(amountToday(habit.today))} ${unit}`.trim();
    return target === null ? amount : `${amount} de ${String(target)} ${unit}`.trim();
  }
  if (isNegative(habit)) {
    return isMet(habit.today) ? 'Hoy, limpio' : 'Hoy hubo una recaída';
  }
  return isMet(habit.today) ? 'Hecho hoy' : 'Todavía no';
}

/**
 * What pressing the control does, said as the thing it does.
 *
 * The accessible name of the row's button, and the same sentence its title carries. A habit
 * to be cut down is not marked when it goes well; it is marked when it goes badly, and a
 * button that said «marcar como hecho» on it would be a button that lied about its effect.
 */
export function markText(habit: HabitSummary): string {
  const name = habit.name;
  if (countsQuantity(habit)) {
    return `Anotar cuánto llevas hoy de ${name}`;
  }
  if (isNegative(habit)) {
    return isMet(habit.today)
      ? `Apuntar una recaída de hoy en ${name}`
      : `Quitar la recaída de hoy en ${name}`;
  }
  return isMet(habit.today) ? `Desmarcar hoy en ${name}` : `Marcar hoy en ${name}`;
}

/**
 * What the row says about the run, which for a weekly habit is the week rather than a run.
 *
 * Both numbers come from the core. Singular and plural are chosen here because that is
 * grammar rather than arithmetic.
 */
export function streakText(habit: HabitSummary): string {
  const { days, weekProgress } = habit.streak;
  if (weekProgress !== null) {
    return `${String(weekProgress.done)} de ${String(weekProgress.target)} esta semana`;
  }
  if (days === 0) {
    return 'Sin racha';
  }
  return days === 1 ? '1 día seguido' : `${String(days)} días seguidos`;
}

/**
 * The list as today's screen shows it: the habits today actually asks something of.
 *
 * Ordering is the core's and is left alone. The person put them in that order and a screen
 * that sorted them again would be a screen overruling them once a day.
 */
export function forToday(habits: readonly HabitSummary[]): readonly HabitSummary[] {
  return habits.filter(isForToday);
}

/**
 * The same list with one habit replaced, in place.
 *
 * Used after marking a day, so that the row redraws without the whole list being asked for
 * again. A habit that is no longer in the list is not put back: the answer arriving after
 * somebody deleted it in another window should not resurrect it on screen.
 */
export function replace(
  habits: readonly HabitSummary[],
  updated: HabitSummary,
): readonly HabitSummary[] {
  return habits.map((habit) => (habit.id === updated.id ? updated : habit));
}

/**
 * The same list with one habit moved one place, up or down.
 *
 * A move on this side only. The core is told once, when the moving is over, with the whole
 * order — telling it after each step would make three presses three writes, and a list that
 * is half reordered is a list somebody else's window could read.
 *
 * A move off either end is not a move. It returns the same list rather than wrapping, because
 * a habit that jumped from the bottom to the top would be a keystroke nobody meant.
 */
export function move(
  habits: readonly HabitSummary[],
  id: string,
  by: -1 | 1,
): readonly HabitSummary[] {
  const from = habits.findIndex((habit) => habit.id === id);
  const to = from + by;
  if (from < 0 || to < 0 || to >= habits.length) {
    return habits;
  }
  const moved = [...habits];
  const [taken] = moved.splice(from, 1);
  if (taken === undefined) {
    return habits;
  }
  moved.splice(to, 0, taken);
  return moved;
}

/**
 * The order to send, which is every habit there is and never a part of one.
 *
 * The core refuses anything else, and it is right to: an order that named half the habits
 * would leave the other half wherever they happened to be, and two windows sending halves
 * would interleave into an order neither of them asked for.
 */
export function orderOf(habits: readonly HabitSummary[]): string[] {
  return habits.map((habit) => habit.id);
}

/**
 * The same list without one habit.
 *
 * Used after putting one away or deleting it. The row goes and the rest stay as they are,
 * rather than the whole list being asked for again over one row that is no longer in it.
 */
export function without(habits: readonly HabitSummary[], id: string): readonly HabitSummary[] {
  return habits.filter((habit) => habit.id !== id);
}
