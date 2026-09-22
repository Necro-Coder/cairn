/**
 * What today's list decides, checked without a browser.
 *
 * Everything here is a reading of a value the core already worked out. So what these cases
 * pin is not arithmetic — there is none — but the two judgements the screen makes on its
 * own: which habits today asks something of, and what each row says about a habit that is
 * read backwards.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { DayState, HabitSummary } from '../../lib/ipc.types.ts';

import {
  amountToday,
  countsQuantity,
  forToday,
  isAtRisk,
  isForToday,
  isMet,
  isNegative,
  markText,
  replace,
  streakText,
  targetToday,
  todayText,
} from './today.ts';

/** A habit with everything ordinary, so each case says only what makes it different. */
function habit(over: Partial<HabitSummary> = {}): HabitSummary {
  return {
    id: '00000000-0000-4000-8000-00000000c001',
    name: 'Meditar',
    icon: null,
    color: null,
    period: 'daily',
    unit: null,
    target: null,
    direction: 'atLeast',
    scheduleMask: 0b111_1111,
    position: 0,
    archived: false,
    todayDay: 20260922,
    today: { state: 'missed', amount: 0, target: 1 },
    streak: { days: 0, atRisk: false, weekProgress: null },
    ...over,
  };
}

/** The five squares, so every case below can be run against all of them. */
const EVERY_STATE: DayState[] = [
  { state: 'done', amount: 1, target: 1 },
  { state: 'missed', amount: 0, target: 1 },
  { state: 'notScheduled', amount: 0 },
  { state: 'extra', amount: 1, target: 1 },
  { state: 'noData' },
];

test('a day the habit never asked about is the one thing kept out of today', () => {
  // Drawing it would make a Sunday look like four failures, which teaches somebody to stop
  // reading the screen. The other four all belong, `noData` included: a habit that has not
  // started yet is a habit worth seeing wait.
  const shown = EVERY_STATE.filter((today) => isForToday(habit({ today }))).map(
    (today) => today.state,
  );

  assert.deepEqual(shown, ['done', 'missed', 'extra', 'noData']);
  assert.equal(isForToday(habit({ today: { state: 'notScheduled', amount: 0 } })), false);
});

test('the list keeps the order the core sent and only drops the days off', () => {
  const list = [
    habit({ id: 'a', name: 'Meditar' }),
    habit({ id: 'b', name: 'Correr', today: { state: 'notScheduled', amount: 0 } }),
    habit({ id: 'c', name: 'Leer', today: { state: 'done', amount: 1, target: 1 } }),
  ];

  assert.deepEqual(
    forToday(list).map((each) => each.name),
    ['Meditar', 'Leer'],
    'the person put them in that order, and a screen that sorted them again would overrule them once a day',
  );
});

test('being at risk is read off what the core said and never worked out again', () => {
  // The core knows what today is in the device's own zone and what the schedule says. This
  // side knows neither, so the only honest thing it can do is carry the answer.
  assert.equal(isAtRisk(habit({ streak: { days: 9, atRisk: true, weekProgress: null } })), true);
  assert.equal(isAtRisk(habit({ streak: { days: 9, atRisk: false, weekProgress: null } })), false);
  assert.equal(isAtRisk(habit({ streak: { days: 0, atRisk: false, weekProgress: null } })), false);
});

test('a day counts as met when it is done or when it went beyond what was asked', () => {
  const met = EVERY_STATE.filter(isMet).map((state) => state.state);

  assert.deepEqual(met, ['done', 'extra']);
});

test('naming a unit is what makes a habit count a quantity, not having a target', () => {
  // A weekly habit has a target too, and there it means how many days of the week rather
  // than how much of anything on one of them.
  assert.equal(countsQuantity(habit({ unit: 'ml', target: 2000 })), true);
  assert.equal(countsQuantity(habit({ unit: null, target: 3, period: 'weekly' })), false);
});

test('what today did and what it asked for are nothing on a day nobody was judged on', () => {
  assert.equal(amountToday({ state: 'noData' }), 0);
  assert.equal(targetToday({ state: 'noData' }), null);
  assert.equal(targetToday({ state: 'notScheduled', amount: 4 }), null);
  assert.equal(amountToday({ state: 'notScheduled', amount: 4 }), 4);
  assert.equal(targetToday({ state: 'missed', amount: 1500, target: 2000 }), 2000);
});

test('a habit to be cut down reads backwards, and says so without anybody looking it up', () => {
  // The good day of «sin azúcar» is the day nobody touched. A row that said «todavía no» on
  // it would be telling somebody they are behind on a day they kept.
  const clean = habit({
    name: 'Sin azúcar',
    direction: 'atMost',
    today: { state: 'done', amount: 0, target: 0 },
  });
  const slipped = habit({
    name: 'Sin azúcar',
    direction: 'atMost',
    today: { state: 'missed', amount: 1, target: 0 },
  });

  assert.equal(isNegative(clean), true);
  assert.equal(todayText(clean), 'Hoy, limpio');
  assert.equal(todayText(slipped), 'Hoy hubo una recaída');
});

test('a habit to be built up reads the ordinary way round', () => {
  assert.equal(todayText(habit({ today: { state: 'done', amount: 1, target: 1 } })), 'Hecho hoy');
  assert.equal(todayText(habit()), 'Todavía no');
});

test('a habit that counts a quantity says how much against how much', () => {
  const water = habit({
    name: 'Beber agua',
    unit: 'ml',
    target: 2000,
    today: { state: 'missed', amount: 1500, target: 2000 },
  });

  assert.equal(todayText(water), '1500 ml de 2000 ml');
});

test('the control says the thing it does, which for a habit to be cut down is a slip', () => {
  // A button that said «marcar como hecho» on «sin azúcar» would be a button lying about its
  // own effect: pressing it records that the day went badly.
  const clean = habit({
    name: 'Sin azúcar',
    direction: 'atMost',
    today: { state: 'done', amount: 0, target: 0 },
  });

  assert.equal(markText(clean), 'Apuntar una recaída de hoy en Sin azúcar');
  assert.equal(markText(habit()), 'Marcar hoy en Meditar');
  assert.equal(
    markText(habit({ today: { state: 'done', amount: 1, target: 1 } })),
    'Desmarcar hoy en Meditar',
  );
  assert.equal(
    markText(habit({ name: 'Beber agua', unit: 'ml', target: 2000 })),
    'Anotar cuánto llevas hoy de Beber agua',
  );
});

test('the run is said in days, and in weeks for a habit judged by the week', () => {
  assert.equal(streakText(habit()), 'Sin racha');
  assert.equal(
    streakText(habit({ streak: { days: 1, atRisk: false, weekProgress: null } })),
    '1 día seguido',
  );
  assert.equal(
    streakText(habit({ streak: { days: 12, atRisk: true, weekProgress: null } })),
    '12 días seguidos',
  );
  assert.equal(
    streakText(
      habit({
        period: 'weekly',
        target: 3,
        streak: { days: 2, atRisk: false, weekProgress: { done: 1, target: 3 } },
      }),
    ),
    '1 de 3 esta semana',
  );
});

test('replacing a row leaves every other one exactly as it was', () => {
  const list = [habit({ id: 'a' }), habit({ id: 'b' }), habit({ id: 'c' })];
  const updated = habit({ id: 'b', name: 'Correr' });

  const after = replace(list, updated);

  assert.deepEqual(
    after.map((each) => each.name),
    ['Meditar', 'Correr', 'Meditar'],
  );
  assert.equal(after[0], list[0], 'the rows that did not change are the same objects');
  assert.equal(after[2], list[2]);
});

test('an answer about a habit that is gone does not put it back on the screen', () => {
  // Somebody deleted it in another window while the reply was in flight. Resurrecting it is
  // worse than losing the update: the row would be there and nothing would remove it.
  const list = [habit({ id: 'a' })];

  assert.deepEqual(replace(list, habit({ id: 'gone' })), list);
});
