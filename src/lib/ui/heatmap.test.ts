/**
 * Where each day of a year lands in the grid, checked without a browser.
 *
 * The arithmetic of fitting three hundred and sixty-six days into fifty-three columns is the
 * kind that is wrong by one for six years out of seven, and wrong by one in a way nobody sees
 * until a particular January. So the years below are chosen for the weekday the first of
 * January falls on rather than for being recent.
 *
 * Nothing here judges a day. Every state comes in from the answer, which is what the core
 * decided, and the only question asked of this module is where the square goes.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { DayCell, DayState, Heatmap } from '../ipc.types.ts';

import {
  DAYS_IN_WEEK,
  MAX_WEEKS,
  daysInYear,
  isLeap,
  place,
  rowOf,
  treatmentOf,
  type Grid,
} from './heatmap.ts';

/** A year the core answered with nothing in it, which is a habit that had not started. */
function blank(year: number): Heatmap {
  return { year, days: [], firstYearWithData: null };
}

/** Every square of a year that the grid actually holds. */
function filled(grid: Grid): NonNullable<Grid['cells'][number]>[] {
  return grid.cells.filter((cell) => cell !== null);
}

test('the grid is always fifty-three columns of seven, whatever the year does', () => {
  // The component draws a fixed grid, because passing a column count through a `style`
  // attribute is a declaration the content security policy drops without saying so.
  for (const year of [2018, 2024, 2026, 2027]) {
    assert.equal(place(blank(year)).cells.length, MAX_WEEKS * DAYS_IN_WEEK);
  }
});

test('a year that starts on a Monday has no gaps in front of it', () => {
  // 2024 opened on a Monday, and it is a leap year, so it is the tightest fit there is.
  assert.equal(rowOf(20240101), 0);

  const grid = place(blank(2024));

  assert.equal(grid.cells[0]?.day, 20240101, 'the first square of the grid is the first day');
  assert.equal(grid.columns, Math.ceil(366 / DAYS_IN_WEEK), 'fifty-three columns, exactly');
  assert.equal(grid.columns, 53);
});

test('a year that starts on a Sunday has six gaps, and its first day is on the Sunday row', () => {
  // 2023 opened on a Sunday, which is the worst case: six squares of nothing in front.
  assert.equal(rowOf(20230101), 6);

  const grid = place(blank(2023));

  for (let row = 0; row < 6; row += 1) {
    assert.equal(grid.cells[row], null, `the square at row ${String(row)} is not a day`);
  }
  assert.equal(grid.cells[6]?.day, 20230101, 'the first of January sits on the Sunday row');
});

test('a leap year places all three hundred and sixty-six days and loses none', () => {
  assert.equal(isLeap(2024), true);
  assert.equal(daysInYear(2024), 366);

  const days = filled(place(blank(2024)));

  assert.equal(days.length, 366);
  assert.equal(days[0]?.day, 20240101);
  assert.equal(days[days.length - 1]?.day, 20241231);
});

test('an ordinary year places all three hundred and sixty-five', () => {
  assert.equal(isLeap(2026), false);
  assert.equal(daysInYear(2026), 365);

  const days = filled(place(blank(2026)));

  assert.equal(days.length, 365);
  assert.equal(days[0]?.day, 20260101);
  assert.equal(days[days.length - 1]?.day, 20261231);
});

test('the centuries that are not leap years are not treated as leap years', () => {
  // The rule everybody writes as `% 4` and nobody tests. 1900 was not a leap year; 2000 was.
  assert.equal(isLeap(1900), false);
  assert.equal(isLeap(2000), true);
  assert.equal(isLeap(2100), false);
});

test('no square holds two days and no day is in two squares', () => {
  for (const year of [2023, 2024, 2025, 2026, 2027]) {
    const grid = place(blank(year));
    const days = filled(grid).map((cell) => cell.day);

    assert.equal(new Set(days).size, days.length, `a day appears twice in ${String(year)}`);
    assert.equal(days.length, daysInYear(year), `${String(year)} does not have all of its days`);
    assert.deepEqual(
      days,
      [...days].sort((a, b) => a - b),
      'the days are not in order',
    );
  }
});

test('every day sits in the row its weekday belongs to', () => {
  // The one property that makes the picture readable: a column is a week, so every square in
  // row nought is a Monday. Wrong by one here and the whole year is sheared.
  const grid = place(blank(2026));

  grid.cells.forEach((cell, at) => {
    if (cell !== null) {
      assert.equal(rowOf(cell.day), at % DAYS_IN_WEEK, `${String(cell.day)} is in the wrong row`);
    }
  });
});

test('a year with all five variants in it draws five different treatments', () => {
  const states: DayState[] = [
    { state: 'done', amount: 1, target: 1 },
    { state: 'missed', amount: 0, target: 1 },
    { state: 'notScheduled', amount: 0 },
    { state: 'extra', amount: 1, target: 1 },
    { state: 'noData' },
  ];
  const days: DayCell[] = states.map((state, at) => ({ day: 20260101 + at, state }));

  const grid = place({ year: 2026, days, firstYearWithData: 2025 });
  const drawn = filled(grid)
    .slice(0, states.length)
    .map((cell) => treatmentOf(cell.state));

  assert.deepEqual(drawn, ['done', 'missed', 'not-scheduled', 'extra', 'no-data']);
  assert.equal(new Set(drawn).size, 5, 'two states are drawn the same way');
});

test('a year the core sent nothing for is still a whole year, all of it with no data', () => {
  // A habit that had not started yet. Drawing a shorter year would be the lie; three hundred
  // and sixty-five squares of «before this existed» is the honest picture.
  const days = filled(place(blank(2026)));

  assert.equal(days.length, 365);
  assert.ok(
    days.every((cell) => cell.state.state === 'noData'),
    'a square the answer said nothing about is anything other than no data',
  );
  assert.ok(days.every((cell) => treatmentOf(cell.state) === 'no-data'));
});

test('a day from another year in the answer is ignored rather than squeezed in', () => {
  // A square in the wrong year is worse than a square missing, and the core has no business
  // sending one — but a grid that quietly moved it would hide the day it did.
  const days: DayCell[] = [
    { day: 20250704, state: { state: 'done', amount: 1, target: 1 } },
    { day: 20260704, state: { state: 'done', amount: 1, target: 1 } },
  ];

  const grid = place({ year: 2026, days, firstYearWithData: 2025 });
  const done = filled(grid).filter((cell) => cell.state.state === 'done');

  assert.deepEqual(
    done.map((cell) => cell.day),
    [20260704],
  );
});

test('the treatment of a state is a reading of it and never a judgement', () => {
  // Nothing here looks at an amount or a target. Whatever the numbers say, the square is
  // drawn by the tag the core put on it, because the core is what weighed them.
  assert.equal(treatmentOf({ state: 'done', amount: 0, target: 9000 }), 'done');
  assert.equal(treatmentOf({ state: 'missed', amount: 9000, target: 1 }), 'missed');
});
