/**
 * Where each day of a year goes in a grid of weeks, and how each one is drawn.
 *
 * Placing days in a grid is the only thing in this module that looks at a calendar, and it
 * is the only thing allowed to: what each day *means* arrives already decided from Rust, in
 * the `DayState` the core put on it. Nothing here judges a day, counts a run or works out
 * whether something was scheduled. Given the same `Heatmap` it always draws the same grid.
 *
 * It is a module rather than logic inside the component because a component cannot be run by
 * `node --test`, and the arithmetic of fitting three hundred and sixty-six days into fifty-
 * three columns is exactly the kind that is wrong by one for six years out of seven.
 */

import type { DayCell, DayState, Heatmap } from '../ipc.types';

/**
 * Fifty-three columns, because a year crosses that many calendar weeks whenever it does not
 * start on a Monday, which is six years out of seven. A grid that lost a column in those
 * years would be a grid that changed shape without warning.
 */
export const MAX_WEEKS = 53;

/** Seven rows, Monday at the top. */
export const DAYS_IN_WEEK = 7;

/**
 * One square of the grid that is a day of the year.
 *
 * Not exported. Anybody reading a cell reaches it through `Grid`, and an export that nobody
 * imports is a name the next person has to decide whether they are allowed to change.
 */
interface Slot {
  /** Which day, as `YYYYMMDD`. */
  readonly day: number;
  /** What the core decided that day says. */
  readonly state: DayState;
}

/**
 * A year laid out in columns of weeks.
 *
 * `cells` is column-major — index `column * 7 + row` — which is the order a grid with
 * `grid-auto-flow: column` reads them in, so the component writes them out in one pass with
 * no index arithmetic of its own. A `null` is a square of the grid that is not a day of this
 * year: the days before the first Monday, and the tail after the thirty-first of December.
 * Those are drawn as nothing at all, which is not the same as a day with no data.
 *
 * It is always fifty-three columns long, whatever `columns` says. The grid the component
 * draws is fixed in CSS, because the alternative is passing a column count through a `style`
 * attribute, which the content security policy drops and the token gate refuses. `columns` is
 * how many of the fifty-three a year actually reaches, which is what the tests assert on.
 */
export interface Grid {
  readonly columns: number;
  readonly cells: readonly (Slot | null)[];
}

/** How each state is drawn. One class per variant, and five that differ without colour. */
export type Treatment = 'done' | 'missed' | 'not-scheduled' | 'extra' | 'no-data';

/** Whether a year has three hundred and sixty-six days. The ordinary rule, written out. */
export function isLeap(year: number): boolean {
  return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
}

/** How many days that year has. */
export function daysInYear(year: number): number {
  return isLeap(year) ? 366 : 365;
}

/**
 * Which row a day belongs in: Monday is nought and Sunday is six.
 *
 * Read in UTC so that no machine's own zone shifts a square by a row. The day number already
 * names a calendar day; turning it into a moment is only a way of asking which weekday that
 * calendar day is, and a local midnight would be a different instant on every machine.
 */
export function rowOf(day: number): number {
  const year = Math.trunc(day / 10_000);
  const month = Math.trunc(day / 100) % 100;
  const date = new Date(Date.UTC(year, month - 1, day % 100));
  return (date.getUTCDay() + 6) % DAYS_IN_WEEK;
}

/** The day `offset` days after another one, as `YYYYMMDD`. */
function dayAfter(day: number, offset: number): number {
  const year = Math.trunc(day / 10_000);
  const month = Math.trunc(day / 100) % 100;
  const date = new Date(Date.UTC(year, month - 1, (day % 100) + offset));
  return date.getUTCFullYear() * 10_000 + (date.getUTCMonth() + 1) * 100 + date.getUTCDate();
}

/**
 * Lays a year out in columns of weeks, oldest week first and Monday at the top.
 *
 * Every day of the year gets a square, whether or not the answer carried one. A year the core
 * sent nothing for is still a year, and drawing three hundred and sixty-five squares of «no
 * data» is the honest picture of a habit that did not exist yet — leaving them out would draw
 * a shorter year instead.
 *
 * A day the answer carries that does not belong to this year is ignored rather than squeezed
 * in somewhere: a square in the wrong year is worse than a square missing.
 */
export function place(heatmap: Heatmap): Grid {
  const byDay = new Map<number, DayCell>();
  for (const cell of heatmap.days) {
    byDay.set(cell.day, cell);
  }

  const firstDay = heatmap.year * 10_000 + 101;
  const lead = rowOf(firstDay);
  const total = daysInYear(heatmap.year);
  const columns = Math.min(MAX_WEEKS, Math.ceil((lead + total) / DAYS_IN_WEEK));

  const cells: (Slot | null)[] = new Array<Slot | null>(MAX_WEEKS * DAYS_IN_WEEK).fill(null);

  for (let index = 0; index < total; index += 1) {
    const slot = lead + index;
    if (slot >= cells.length) {
      break;
    }
    const day = dayAfter(firstDay, index);
    // Column-major, so that the component can write the array out in the order a grid with
    // `grid-auto-flow: column` reads it.
    const at = Math.trunc(slot / DAYS_IN_WEEK) * DAYS_IN_WEEK + (slot % DAYS_IN_WEEK);
    cells[at] = { day, state: byDay.get(day)?.state ?? { state: 'noData' } };
  }

  return { columns, cells };
}

/**
 * How one square is drawn, which is a reading of what the core said and never a judgement.
 *
 * Five variants and five treatments, and the treatments differ by shape as well as by colour:
 * a filled square, a filled square with a hole in it, an outlined one, a dot on its own and a
 * dashed outline. Somebody who cannot tell the two greens apart can still tell the five apart,
 * which is the whole reason the shapes are not all squares.
 */
export function treatmentOf(state: DayState): Treatment {
  switch (state.state) {
    case 'done':
      return 'done';
    case 'missed':
      return 'missed';
    case 'notScheduled':
      return 'not-scheduled';
    case 'extra':
      return 'extra';
    case 'noData':
      return 'no-data';
    default: {
      const unreachable: never = state;
      return unreachable;
    }
  }
}
