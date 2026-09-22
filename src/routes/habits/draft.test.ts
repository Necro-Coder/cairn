/**
 * The turn between what the form holds and what the core is sent, checked without a browser.
 *
 * The case that matters most is the round trip: a habit read into the form and sent straight
 * back out has to be the same habit. The field that quietly does not survive that trip is the
 * expensive defect of this screen — somebody edits a name and loses a schedule, and nothing
 * says so until a streak breaks a week later — and it is invisible to every other test here.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { FieldProblem, HabitDetail } from '../../lib/ipc.types.ts';

import {
  EVERY_DAY,
  WEEKDAYS,
  blankForm,
  countsQuantity,
  daysOf,
  draftOf,
  formOf,
  fromDateInput,
  maskOf,
  problemText,
  problemsByField,
  toDateInput,
} from './draft.ts';

/** Seven booleans from the weekdays that are on, named as the screen names them. */
function onlyOn(...names: string[]): boolean[] {
  return WEEKDAYS.map((day) => names.includes(day));
}

/** A habit as the core answers with one, so each case says only what makes it different. */
function habit(over: Partial<HabitDetail> = {}): HabitDetail {
  return {
    id: '00000000-0000-4000-8000-00000000d001',
    name: 'Meditar',
    icon: null,
    color: null,
    period: 'daily',
    unit: null,
    target: null,
    direction: 'atLeast',
    scheduleMask: EVERY_DAY,
    position: 0,
    archived: false,
    todayDay: 20260922,
    today: { state: 'missed', amount: 0, target: 1 },
    streak: { days: 0, atRisk: false, weekProgress: null },
    notes: null,
    startedOn: 20240301,
    aggregation: 'sum',
    ...over,
  };
}

test('a blank form becomes the draft the product agreed on, with no days chosen', () => {
  const draft = draftOf(blankForm(20260922));

  assert.deepEqual(draft, {
    name: '',
    notes: null,
    icon: null,
    color: null,
    period: 'daily',
    unit: null,
    target: null,
    aggregation: 'sum',
    direction: 'atLeast',
    // Nought, not one hundred and twenty-seven. The core reads nought as every day, and the
    // screen says so out loud; sending the seven bits would make the form disagree with the
    // sentence under it.
    scheduleMask: 0,
    startedOn: 20260922,
  });
});

test('all seven days chosen is the whole week', () => {
  const form = blankForm(20260922);
  form.days = WEEKDAYS.map(() => true);

  assert.equal(draftOf(form).scheduleMask, 0b111_1111);
  assert.equal(draftOf(form).scheduleMask, 127);
});

test('Monday, Wednesday and Friday, with Monday in the lowest bit', () => {
  const form = blankForm(20260922);
  form.days = onlyOn('Lunes', 'Miércoles', 'Viernes');

  // Bits nought, two and four. Getting the order wrong here shifts somebody's whole week.
  assert.equal(draftOf(form).scheduleMask, 0b001_0101);
  assert.equal(draftOf(form).scheduleMask, 21);
});

test('Monday alone is one, and Sunday alone is sixty-four', () => {
  const monday = blankForm(20260922);
  monday.days = onlyOn('Lunes');
  const sunday = blankForm(20260922);
  sunday.days = onlyOn('Domingo');

  assert.equal(maskOf(monday.days), 1);
  assert.equal(maskOf(sunday.days), 64);
});

test('a mask turned into boxes and back is the same mask', () => {
  for (let mask = 1; mask <= EVERY_DAY; mask += 1) {
    assert.equal(maskOf(daysOf(mask)), mask, `the mask ${String(mask)} did not survive`);
  }
});

test('nought as a mask comes back as every box ticked, because that is what it means', () => {
  assert.deepEqual(
    daysOf(0),
    WEEKDAYS.map(() => true),
  );
});

test('a day travels to a date control and back unchanged', () => {
  assert.equal(toDateInput(20260922), '2026-09-22');
  assert.equal(toDateInput(20240101), '2024-01-01');
  assert.equal(fromDateInput('2026-09-22'), 20260922);
  assert.equal(fromDateInput(toDateInput(20241231)), 20241231);
});

test('a date the control could not have produced is nought and never a guess', () => {
  assert.equal(fromDateInput(''), 0);
  assert.equal(fromDateInput('hoy'), 0);
  assert.equal(fromDateInput('2026-9-2'), 0);
});

test('a habit read into the form and sent straight back out is the same habit', () => {
  // The one that catches the field lost while editing. Every shape a habit can have, because
  // the field that goes missing is always the one the obvious case does not exercise.
  const shapes: HabitDetail[] = [
    habit(),
    habit({ notes: 'Diez minutos.', icon: 'flor', color: 'azul' }),
    habit({ period: 'weekly', target: 3, scheduleMask: 0b001_0101 }),
    habit({ unit: 'ml', target: 2000, aggregation: 'sum' }),
    habit({ unit: 'páginas', target: 30, aggregation: 'highest', direction: 'atMost' }),
    habit({ direction: 'atMost', startedOn: 20200229 }),
    habit({ scheduleMask: 0b100_0000, startedOn: 19991231 }),
  ];

  for (const original of shapes) {
    const draft = draftOf(formOf(original));

    assert.deepEqual(
      draft,
      {
        name: original.name,
        notes: original.notes,
        icon: original.icon,
        color: original.color,
        period: original.period,
        unit: original.unit,
        target: original.target,
        aggregation: original.aggregation,
        direction: original.direction,
        scheduleMask: original.scheduleMask,
        startedOn: original.startedOn,
      },
      `${original.name} lost something on the way through the form`,
    );
  }
});

test('naming a unit is what makes a habit count a quantity', () => {
  const form = blankForm(20260922);

  assert.equal(countsQuantity(form), false);

  form.unit = 'ml';
  assert.equal(countsQuantity(form), true);

  form.unit = '   ';
  assert.equal(countsQuantity(form), false, 'spaces are not a unit');
});

test('a target typed into a habit that counts nothing by the day is not sent', () => {
  // The screen removes the field rather than disabling it, so there is nothing on screen to
  // disagree with what is sent; this makes sure a value left behind goes with it.
  const form = blankForm(20260922);
  form.target = '2000';

  assert.equal(draftOf(form).target, null);
  assert.equal(draftOf(form).unit, null);
});

test('a weekly habit keeps its target even with no unit, because there it is days a week', () => {
  const form = blankForm(20260922);
  form.period = 'weekly';
  form.target = '3';

  assert.equal(draftOf(form).target, 3);
  assert.equal(draftOf(form).unit, null);
});

test('text with nothing in it is nothing, not an empty string for the core to store', () => {
  const form = blankForm(20260922);
  form.notes = '   ';
  form.icon = '';
  form.color = '\n';

  const draft = draftOf(form);

  assert.equal(draft.notes, null);
  assert.equal(draft.icon, null);
  assert.equal(draft.color, null);
});

test('every problem is filed under its own field, and none is lost on the way', () => {
  const problems: FieldProblem[] = [
    { field: 'name', code: 'empty' },
    { field: 'target', code: 'negative' },
    { field: 'name', code: 'tooLong' },
    { field: 'startedOn', code: 'notADay' },
  ];

  const byField = problemsByField(problems);

  assert.deepEqual([...byField.keys()].sort(), ['name', 'startedOn', 'target']);
  assert.deepEqual(byField.get('name'), ['empty', 'tooLong'], 'a field keeps both of its problems');
  assert.deepEqual(byField.get('target'), ['negative']);
  assert.deepEqual(byField.get('startedOn'), ['notADay']);
  assert.equal(
    [...byField.values()].flat().length,
    problems.length,
    'a problem was dropped between the core and the field it belongs to',
  );
});

test('a form the core found nothing wrong with has nothing filed against it', () => {
  assert.equal(problemsByField([]).size, 0);
});

test('every problem gets a sentence, and one nobody has seen before still gets one', () => {
  for (const code of ['empty', 'tooLong', 'negative', 'notADay']) {
    const said = problemText('name', code);

    assert.ok(said.length > 0);
    assert.ok(!said.includes(code), `"${said}" shows the core's own word for it`);
  }

  // The core is allowed to grow a reason this build has never seen. Printing it raw at
  // somebody would be worse than saying plainly that what is there is not accepted.
  const unknown = problemText('target', 'somethingNewEntirely');

  assert.ok(unknown.length > 0);
  assert.ok(!unknown.includes('somethingNewEntirely'));
});
