/**
 * What the detail screen decides for itself, checked without a browser.
 *
 * Two things, and neither is arithmetic about habits: how far the year arrows may go, and how
 * a part over a whole is written. An arrow available one year too far is a defect nobody
 * notices until January, which is exactly the kind worth pinning here.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { Ratio, Streak } from '../../lib/ipc.types.ts';

import { canGoBack, canGoForward, daysText, ratioText, weekText, yearOf } from './detail.ts';

/** A run with nothing special about it, so each case says only what makes it different. */
function streak(over: Partial<Streak> = {}): Streak {
  return { days: 0, atRisk: false, weekProgress: null, ...over };
}

test('the year behind is bounded by the earliest year anything was marked in', () => {
  assert.equal(canGoBack(2026, 2024), true);
  assert.equal(canGoBack(2025, 2024), true);
  assert.equal(canGoBack(2024, 2024), false, 'there is nothing before the first year');
  assert.equal(canGoBack(2023, 2024), false, 'and nothing before that either');
});

test('a habit with nothing marked at all has no year behind it', () => {
  // The arrow is unavailable rather than absent. A control that disappears is a control
  // somebody looks for and then wonders whether they imagined.
  assert.equal(canGoBack(2026, null), false);
});

test('the year ahead is bounded by the year today is in', () => {
  assert.equal(canGoForward(2025, 2026), true);
  assert.equal(canGoForward(2026, 2026), false, 'this year is as far ahead as there is');
  assert.equal(canGoForward(2027, 2026), false, 'and a year that has not started is further');
});

test('which year a day belongs to is read off the day the core sent', () => {
  // The core's own today, carried on the square it judged against. A `Date` here would be a
  // second opinion about where the year ends.
  assert.equal(yearOf(20260922), 2026);
  assert.equal(yearOf(20240101), 2024);
  assert.equal(yearOf(20251231), 2025);
});

test('a part over a whole is written with both numbers and the percentage behind them', () => {
  // The percentage alone is a figure nobody can act on: three days missed out of thirty is a
  // different month from one missed out of ten, and both round to the same neighbourhood.
  assert.equal(ratioText({ done: 27, of: 30, percent: 90 }), '27 de 30 (90%)');
  assert.equal(ratioText({ done: 1, of: 10, percent: 10 }), '1 de 10 (10%)');
  assert.equal(ratioText({ done: 30, of: 30, percent: 100 }), '30 de 30 (100%)');
  assert.equal(ratioText({ done: 0, of: 5, percent: 0 }), '0 de 5 (0%)');
});

test('a month with nothing to count yet is said in words and never as zero per cent', () => {
  // Nothing divides by anything here — the core sent the percentage — but reporting a first
  // day as «0%» would call a perfect month a total failure before it had a chance.
  const nothing: Ratio = { done: 0, of: 0, percent: 0 };

  assert.equal(ratioText(nothing), 'Todavía no hay días que contar este mes');
  assert.ok(!ratioText(nothing).includes('0%'));
});

test('the percentage carried is the one written, never one worked out again here', () => {
  // Deliberately inconsistent numbers. If this ever divided, it would say 50 and disagree
  // with every other screen that shows the same month.
  assert.equal(ratioText({ done: 1, of: 2, percent: 99 }), '1 de 2 (99%)');
});

test('the week in progress is said so that it does not read as a run about to break', () => {
  const weekly = streak({ days: 4, weekProgress: { done: 1, target: 3 } });

  const said = weekText(weekly);

  assert.ok(said !== null);
  assert.ok(said.includes('1 de 3'));
  assert.ok(said.includes('abierta'), 'a weekly habit at one of three on a Tuesday is not behind');
});

test('a habit judged by the day has no week to say anything about', () => {
  assert.equal(weekText(streak({ days: 9 })), null);
});

test('a run of days is said as days, singular and plural and none', () => {
  assert.equal(daysText(0), 'Sin racha');
  assert.equal(daysText(1), '1 día');
  assert.equal(daysText(2), '2 días');
  assert.equal(daysText(365), '365 días');
});
