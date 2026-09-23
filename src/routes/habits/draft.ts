/**
 * What the form holds on screen, and how it becomes the draft the core is sent.
 *
 * Out of the component for one reason above the others: the round trip. A habit read into the
 * form and sent straight back out has to be the same habit, and the field that quietly does
 * not survive that trip is the expensive defect of this screen — somebody edits a name and
 * loses a schedule, and nothing says so until a streak breaks a week later.
 *
 * Nothing here decides whether a habit is acceptable. The form stops what a control can stop
 * — a number field does not accept letters — and the core says what a habit may be. Two sets
 * of rules end up disagreeing, and the one that loses is always the one further from the data.
 */

import type {
  FieldProblem,
  HabitAggregation,
  HabitDetail,
  HabitDirection,
  HabitDraft,
  HabitPeriod,
} from '../../lib/ipc.types';

/** Seven days, Monday first, which is the order they are drawn in and the order of the bits. */
export const WEEKDAYS = [
  'Lunes',
  'Martes',
  'Miércoles',
  'Jueves',
  'Viernes',
  'Sábado',
  'Domingo',
] as const;

/** All seven bits set, which is what the core reads an empty mask as. */
export const EVERY_DAY = 0b111_1111;

/**
 * What the form is holding, in the shapes the controls actually have.
 *
 * Text where a control gives text, even for numbers: an `<input type="number">` hands back a
 * string, and a form that pretended otherwise would have to invent a number for a half-typed
 * one. The turn into a draft is where a string becomes a number or becomes nothing.
 */
export interface FormState {
  name: string;
  notes: string;
  icon: string;
  color: string;
  period: HabitPeriod;
  /** Naming a unit is what makes a habit count a quantity. Empty means it does not. */
  unit: string;
  target: string;
  aggregation: HabitAggregation;
  direction: HabitDirection;
  /** One per weekday, Monday first. All false means every day, which the screen says out loud. */
  days: boolean[];
  /** The first day it is judged on, as the date control gives it: `YYYY-MM-DD`. */
  startedOn: string;
}

/** A day number as `YYYYMMDD` written the way a date control wants it. */
export function toDateInput(day: number): string {
  const year = Math.trunc(day / 10_000);
  const month = Math.trunc(day / 100) % 100;
  return `${String(year).padStart(4, '0')}-${String(month).padStart(2, '0')}-${String(day % 100).padStart(2, '0')}`;
}

/** The other way round. A value the control cannot produce comes back as nought. */
export function fromDateInput(value: string): number {
  const parts = /^(\d{4})-(\d{2})-(\d{2})$/u.exec(value);
  if (parts === null) {
    return 0;
  }
  return Number(parts[1]) * 10_000 + Number(parts[2]) * 100 + Number(parts[3]);
}

/**
 * The seven checkboxes as the mask the core stores, Monday in the lowest bit.
 *
 * None of them ticked is nought, and nought is what the core reads as every day. It is not
 * corrected here: the screen says what nothing selected means, and sending the seven bits
 * instead would make the form disagree with the sentence under it.
 */
export function maskOf(days: readonly boolean[]): number {
  return days.reduce((mask, on, at) => (on ? mask | (1 << at) : mask), 0);
}

/** The mask back into seven checkboxes. The core never stores nought, but a draft may hold it. */
export function daysOf(mask: number): boolean[] {
  const effective = mask === 0 ? EVERY_DAY : mask;
  return WEEKDAYS.map((_each, at) => (effective & (1 << at)) !== 0);
}

/** A form with nothing in it, for a habit that does not exist yet. */
export function blankForm(today: number): FormState {
  return {
    name: '',
    notes: '',
    icon: '',
    color: '',
    period: 'daily',
    unit: '',
    target: '',
    aggregation: 'sum',
    direction: 'atLeast',
    // Nothing selected, which the screen says means every day. Starting with all seven ticked
    // would be the same habit and a different first impression: it would look like a choice
    // somebody had made rather than the default.
    days: WEEKDAYS.map(() => false),
    startedOn: toDateInput(today),
  };
}

/** A habit read into the form, so that editing starts from what is there. */
export function formOf(habit: HabitDetail): FormState {
  return {
    name: habit.name,
    notes: habit.notes ?? '',
    icon: habit.icon ?? '',
    color: habit.color ?? '',
    period: habit.period,
    unit: habit.unit ?? '',
    target: habit.target === null ? '' : String(habit.target),
    aggregation: habit.aggregation,
    direction: habit.direction,
    days: daysOf(habit.scheduleMask),
    startedOn: toDateInput(habit.startedOn),
  };
}

/** Whether what is on screen describes a habit that counts a quantity. */
export function countsQuantity(form: FormState): boolean {
  return form.unit.trim() !== '';
}

/** Text with nothing in it is nothing, not an empty string the core would have to store. */
function orNothing(value: string): string | null {
  const trimmed = value.trim();
  return trimmed === '' ? null : trimmed;
}

/**
 * The form as the draft the core is sent.
 *
 * The three fields that only belong to a habit counting a quantity — unit, target and
 * aggregation — are sent as they are when there is a unit and as their absent values when
 * there is not. The screen removes those controls rather than disabling them, so there is
 * nothing on screen to disagree with what is sent.
 *
 * A weekly habit keeps its target: there it is how many days of the week rather than how much
 * of anything on one of them, which is why the target is not tied to the unit for it.
 */
export function draftOf(form: FormState): HabitDraft {
  const quantity = countsQuantity(form);
  const wantsTarget = quantity || form.period === 'weekly';
  const target = Number.parseInt(form.target, 10);
  return {
    name: form.name.trim(),
    notes: orNothing(form.notes),
    icon: orNothing(form.icon),
    color: orNothing(form.color),
    period: form.period,
    unit: quantity ? form.unit.trim() : null,
    target: wantsTarget && !Number.isNaN(target) ? target : null,
    aggregation: form.aggregation,
    direction: form.direction,
    scheduleMask: maskOf(form.days),
    startedOn: fromDateInput(form.startedOn),
  };
}

/**
 * Every problem the core found, filed under the field it is about.
 *
 * A map rather than a list, because each one is drawn beside its own control: a column of
 * complaints above a form makes somebody read all of them to find out which one is theirs.
 * A field with more than one problem keeps both, in the order the core sent them.
 */
export function problemsByField(problems: readonly FieldProblem[]): Map<string, string[]> {
  const byField = new Map<string, string[]>();
  for (const problem of problems) {
    const already = byField.get(problem.field);
    if (already === undefined) {
      byField.set(problem.field, [problem.code]);
    } else {
      already.push(problem.code);
    }
  }
  return byField;
}

/**
 * The Spanish sentence for one problem with one field.
 *
 * Written here and nowhere else, like the error sentences in `async.ts`. An unknown code is
 * said as plainly as it can be rather than shown as itself: the core is allowed to grow a
 * reason this build has never seen, and a screen printing `notADay` at somebody is worse than
 * a screen saying it does not accept what is there.
 */
export function problemText(field: string, code: string): string {
  switch (code) {
    case 'empty':
      return 'Hace falta un nombre.';
    case 'tooLong':
      return 'Es demasiado largo.';
    case 'negative':
      return 'No puede ser negativo.';
    case 'tooLarge':
      return 'Es un número demasiado grande.';
    case 'notANumber':
      return 'Tiene que ser un número.';
    case 'notADay':
      return 'Esa fecha no existe.';
    case 'missing':
      return 'Falta rellenarlo.';
    case 'notAllowed':
      return 'Eso no vale para este tipo de hábito.';
    case 'outOfRange':
      return 'Está fuera de lo que se acepta.';
    default:
      return `No se acepta lo que hay en «${field}».`;
  }
}
