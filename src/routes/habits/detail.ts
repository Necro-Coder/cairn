/**
 * The two things the detail screen works out for itself, and nothing else.
 *
 * Which years the arrows may reach, and how a part over a whole is written. Neither is a
 * calculation about habits: the streak, the record and the percentage all arrive from Rust
 * already worked out, percentage included, so that a second rounding rule on this side cannot
 * disagree with the first about the same two numbers.
 *
 * Out of the component so that `node --test` can reach it, for the usual reason: an arrow
 * that is available one year too far is a defect nobody notices until January.
 */

import type { Ratio, Streak } from '../../lib/ipc.types';

/**
 * Whether there is an earlier year to go to.
 *
 * Bounded by the earliest year the habit has a mark in, which the core sends with the year
 * itself. A habit with no marks at all has no earlier year, and the arrow is unavailable
 * rather than hidden: a control that disappears is a control somebody looks for.
 */
export function canGoBack(year: number, firstYearWithData: number | null): boolean {
  return firstYearWithData !== null && year > firstYearWithData;
}

/**
 * Whether there is a later year to go to.
 *
 * Bounded by the year today is in. There is nothing to show in a year that has not happened,
 * and an arrow into it would answer with three hundred and sixty-five squares of nothing.
 */
export function canGoForward(year: number, thisYear: number): boolean {
  return year < thisYear;
}

/** Which year a `YYYYMMDD` belongs to. The core's own today is where this side gets it from. */
export function yearOf(day: number): number {
  return Math.trunc(day / 10_000);
}

/**
 * A part over a whole, with both numbers and the percentage the core rounded.
 *
 * Both numbers, never the percentage alone: «90%» of an unknown number of days is a figure
 * nobody can act on, and three days missed out of thirty is a different month from one missed
 * out of ten.
 *
 * A whole of nothing is said in words. It is not zero per cent — nothing has been asked yet —
 * and writing it as zero would report a perfect month as a total failure on its first day.
 */
export function ratioText(ratio: Ratio): string {
  if (ratio.of === 0) {
    return 'Todavía no hay días que contar este mes';
  }
  return `${String(ratio.done)} de ${String(ratio.of)} (${String(ratio.percent)}%)`;
}

/**
 * How the week in progress is going, said so that it does not read as a run about to break.
 *
 * A weekly habit at one of three on a Tuesday is not behind. The sentence says the week is
 * still open, because the number on its own invites reading it as a failure in progress.
 */
export function weekText(streak: Streak): string | null {
  const { weekProgress } = streak;
  if (weekProgress === null) {
    return null;
  }
  return `${String(weekProgress.done)} de ${String(weekProgress.target)} esta semana, que sigue abierta`;
}

/** A run of days, said as days. The number is the core's; only the grammar is here. */
export function daysText(days: number): string {
  if (days === 0) {
    return 'Sin racha';
  }
  return days === 1 ? '1 día' : `${String(days)} días`;
}
