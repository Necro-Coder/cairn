/**
 * The four states a screen that asks the core for something can be in, and no others.
 *
 * This is the first module of the product, and what is decided here gets copied three times:
 * habits now, passwords in phase 06, finances in phase 07. Three screens carrying loose
 * `loading` and `error` flags become nine, and the combinations nobody meant — loading and
 * failed at once, ready with nothing in it — appear by themselves, one reasonable-looking
 * line at a time. A discriminated union makes them impossible to write rather than merely
 * unlikely, which is the only kind of guarantee worth having across nine screens.
 *
 * `empty` is a state of its own rather than a `ready` holding nothing, because the two are
 * drawn by different things: `ready` is the screen, and `empty` is the first day, which the
 * design system says is real content and not a placeholder.
 *
 * Nothing here runs a timer. Waiting to draw the waiting indicator is the container's job
 * and it does it in CSS, so that this module stays plain data and can be read by `node --test`
 * without a browser anywhere near it.
 */

import type { HabitsError } from '../ipc.types';

/**
 * What a screen is doing, as one value.
 *
 * A union rather than two booleans, because two booleans have four combinations and only
 * three of them mean anything.
 */
export type Async<T> =
  | { readonly status: 'loading' }
  | { readonly status: 'empty' }
  | { readonly status: 'ready'; readonly value: T }
  | { readonly status: 'failed'; readonly error: HabitsError };

/** Asked, and nothing back yet. */
export const loading = <T>(): Async<T> => ({ status: 'loading' });

/** Answered, and there is nothing there yet. The first day, not a fault. */
export const empty = <T>(): Async<T> => ({ status: 'empty' });

/** Answered, with something to draw. */
export const ready = <T>(value: T): Async<T> => ({ status: 'ready', value });

/** Refused, with the reason the core gave. */
export const failed = <T>(error: HabitsError): Async<T> => ({ status: 'failed', error });

/**
 * Whether a rejected value is one of the core's errors rather than something else entirely.
 *
 * The core rejects a command with the tagged object it serialised, never with an `Error`. So
 * anything arriving here with a `kind` this module knows is the core answering; anything else
 * is a mistake on this side of the boundary — a module that failed to load, a property read
 * off nothing — and is not allowed to be reported as though the database had refused.
 */
const ERROR_KINDS = [
  'locked',
  'notFound',
  'invalid',
  'dayInFuture',
  'dayTooOld',
  'incompleteOrder',
  'noZone',
  'storage',
] as const;

/** Narrows an unknown rejection to a {@link HabitsError}, or says it is not one. */
function asHabitsError(thrown: unknown): HabitsError | null {
  if (typeof thrown !== 'object' || thrown === null || !('kind' in thrown)) {
    return null;
  }
  // The `in` check above is what TypeScript needs to read `.kind`; the list is what decides.
  // The assertion is the one thing narrowing cannot express: having proved the tag is one of
  // the eight, the object is the variant that tag belongs to.
  const { kind } = thrown;
  return ERROR_KINDS.some((known) => known === kind) ? (thrown as HabitsError) : null;
}

/**
 * Turns a promise into the four states, deciding `empty` with the predicate given.
 *
 * The predicate is a parameter rather than a check for an empty array, because what counts
 * as nothing is the screen's business: a list is empty when it has no rows, and a year of
 * squares is never empty even when every one of them is blank.
 *
 * A rejection that is not one of the core's errors is reported as `storage` rather than
 * rethrown. Not to hide it — the console still has it, because it is genuinely a defect on
 * this side — but because a promise rejecting into a component leaves the screen in whatever
 * state it was in, which is the one outcome worse than an honest error panel.
 */
export async function load<T>(
  work: () => Promise<T>,
  isEmpty: (value: T) => boolean,
): Promise<Async<T>> {
  try {
    const value = await work();
    return isEmpty(value) ? empty<T>() : ready(value);
  } catch (thrown: unknown) {
    const error = asHabitsError(thrown);
    if (error === null) {
      // Not the core answering. Kept where somebody repairing a machine will find it, and
      // reported to the person as the one thing that is true: it did not work.
      // eslint-disable-next-line no-console -- the one place a defect on this side has to leave a trace: it is reported to the person as `storage`, which is true but says nothing about the cause, and swallowing it entirely is the fallback that hides the fault. There is no logger in this application; the console is what a WebView has.
      console.error('the boundary rejected with something that is not a core error', thrown);
      return failed<T>({ kind: 'storage' });
    }
    return failed<T>(error);
  }
}

/**
 * The Spanish sentence for an error. The only place any of them is written.
 *
 * Exhaustive over the union, with the unreachable branch typed as `never`: adding a variant
 * to `HabitsError` without a sentence for it stops the typecheck rather than reaching a
 * screen as a blank panel.
 *
 * `noZone` has a sentence of its own and says what to do, because it is the only one of the
 * eight the person can fix without anybody's help. `locked` has one too, although the closed
 * vault has a screen of its own and this should never be read: an error with no sentence is
 * worse than one that arrives out of turn.
 */
export function messageFor(error: HabitsError): string {
  switch (error.kind) {
    case 'locked':
      return 'La caja fuerte está cerrada. Ábrela para ver tus hábitos.';
    case 'notFound':
      return 'Ese hábito ya no está. Puede que lo borraras en otra ventana.';
    case 'invalid':
      return 'Hay algo que corregir en el formulario. Está marcado debajo de cada campo.';
    case 'dayInFuture':
      return 'Ese día todavía no ha llegado. Solo se pueden marcar días que ya han pasado.';
    case 'dayTooOld':
      return 'Ese día queda más atrás de lo que se puede marcar.';
    case 'incompleteOrder':
      return 'El orden ha cambiado mientras lo movías. Vuelve a cargar la lista e inténtalo otra vez.';
    case 'noZone':
      return 'El sistema no dice en qué zona horaria estás, y sin eso no hay un «hoy» que contar. Configura la zona horaria del equipo y vuelve a entrar.';
    case 'storage':
      return 'No se ha podido leer ni escribir en la base de datos.';
    default: {
      const unreachable: never = error;
      return unreachable;
    }
  }
}
