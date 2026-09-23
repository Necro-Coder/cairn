/**
 * The four states, checked without a browser.
 *
 * Run by `node --test`, which strips the types and executes the module directly, so `async.ts`
 * holds no runes and imports `HabitsError` as a type: everything that survives to runtime here
 * is plain JavaScript over plain objects.
 *
 * The case the table calls number six — a `ready` written without a value — is not here and
 * cannot be: it is a compile error, so a file containing it would stop `npm run check` rather
 * than fail a test. It was checked by hand once, by writing
 *
 *     const wrong: Async<number> = { status: 'ready' };
 *
 * which the typecheck rejected with "Property 'value' is missing in type", and then undone.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { HabitsError } from '../ipc.types.ts';

import { empty, failed, load, loading, messageFor, ready, type Async } from './async.ts';

/** Every variant of the error union, so the sentences can be checked as a set. */
const EVERY_ERROR: HabitsError[] = [
  { kind: 'locked' },
  { kind: 'notFound' },
  { kind: 'invalid', problems: [{ field: 'name', code: 'empty' }] },
  { kind: 'dayInFuture' },
  { kind: 'dayTooOld' },
  { kind: 'incompleteOrder' },
  { kind: 'noZone' },
  { kind: 'storage' },
];

/** A list is nothing when it has no rows, which is what most screens mean by empty. */
const noRows = (value: readonly unknown[]): boolean => value.length === 0;

test('the four constructors say exactly what they are and nothing else', () => {
  assert.deepEqual(loading<number>(), { status: 'loading' });
  assert.deepEqual(empty<number>(), { status: 'empty' });
  assert.deepEqual(ready(7), { status: 'ready', value: 7 });
  assert.deepEqual(failed<number>({ kind: 'notFound' }), {
    status: 'failed',
    error: { kind: 'notFound' },
  });
});

test('something that resolves with content is ready, carrying that content', async () => {
  const state = await load(() => Promise.resolve([1, 2, 3]), noRows);

  assert.deepEqual(state, { status: 'ready', value: [1, 2, 3] });
});

test('something that resolves with nothing is empty, and never a ready holding nothing', async () => {
  const state = await load(() => Promise.resolve([]), noRows);

  assert.deepEqual(state, { status: 'empty' });
});

test('what counts as nothing is the screen, not this module', async () => {
  // A year of squares is never empty, even when every one of them is blank. The predicate is
  // a parameter precisely so that this module never has to hold that opinion.
  const neverEmpty = await load(
    () => Promise.resolve([]),
    () => false,
  );

  assert.deepEqual(neverEmpty, { status: 'ready', value: [] });
});

test('a rejection from the core is carried through as it arrived', async () => {
  const error: HabitsError = { kind: 'invalid', problems: [{ field: 'target', code: 'range' }] };

  // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- rejecting with the plain tagged object is exactly what Tauri does with what the core serialised, and a test that rejected with an Error would be testing a shape the boundary never produces
  const state = await load(() => Promise.reject(error), noRows);

  assert.deepEqual(state, { status: 'failed', error });
});

test('every one of the core errors survives the trip with its own tag', async () => {
  for (const error of EVERY_ERROR) {
    // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see the note above; the reason is the same one
    const state = await load(() => Promise.reject(error), noRows);

    assert.deepEqual(state, { status: 'failed', error });
  }
});

test('a rejection that is not one of the core errors becomes storage, and never throws', async () => {
  // Three shapes that are not the core answering: a real `Error`, something with a tag that
  // is not one of the eight, and nothing at all. None of them may reach a screen as a throw,
  // because a promise rejecting into a component leaves it in whatever state it was in.
  const notCore: unknown[] = [
    new TypeError('read of undefined'),
    { kind: 'somethingElse' },
    null,
    'a string',
  ];

  for (const thrown of notCore) {
    // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- these four are the point of the test: what happens when the rejection is not the core answering
    const state = await load(() => Promise.reject(thrown), noRows);

    assert.deepEqual(state, { status: 'failed', error: { kind: 'storage' } });
  }
});

test('a synchronous throw inside the work is caught too', async () => {
  const state = await load(() => {
    throw new Error('the module did not load');
  }, noRows);

  assert.deepEqual(state, { status: 'failed', error: { kind: 'storage' } });
});

test('the eight errors have eight different sentences, all of them in Spanish', () => {
  const sentences = EVERY_ERROR.map(messageFor);

  assert.equal(new Set(sentences).size, EVERY_ERROR.length, 'two errors share a sentence');
  for (const sentence of sentences) {
    assert.ok(sentence.length > 0, 'an error has no sentence');
    assert.ok(/[áéíóúñ¿¡]/u.test(sentence) || /\b(la|el|no|ese|hay)\b/u.test(sentence));
  }
});

test('no sentence leaks an identifier from the other side of the boundary', () => {
  // The tags are what the core speaks. A sentence containing one is a sentence written for
  // whoever wrote the core rather than for whoever reads the screen.
  for (const error of EVERY_ERROR) {
    const sentence = messageFor(error);

    for (const tag of EVERY_ERROR.map((each) => each.kind)) {
      assert.ok(!sentence.includes(tag), `"${sentence}" contains the tag ${tag}`);
    }
  }
});

test('the vault being closed says what is closed and what opens it', () => {
  // The one error with a screen of its own already. The sentence exists so that arriving out
  // of turn is still readable, and it has to name the thing it names everywhere else.
  assert.match(messageFor({ kind: 'locked' }), /caja fuerte/u);
});

test('the time zone error is its own sentence and says what to do about it', () => {
  const zone = messageFor({ kind: 'noZone' });

  assert.notEqual(zone, messageFor({ kind: 'storage' }));
  assert.match(zone, /zona horaria/u);
});

test('a state is one of four things and the compiler is what says so', () => {
  // Narrowing on the tag is what every screen does, and it is the whole point of the union:
  // `value` is reachable only inside `ready`, and `error` only inside `failed`.
  const states: Async<number>[] = [loading(), empty(), ready(4), failed({ kind: 'storage' })];

  const described = states.map((state) => {
    switch (state.status) {
      case 'loading':
        return 'waiting';
      case 'empty':
        return 'nothing yet';
      case 'ready':
        return `got ${state.value}`;
      case 'failed':
        return `refused: ${state.error.kind}`;
      default: {
        const unreachable: never = state;
        return unreachable;
      }
    }
  });

  assert.deepEqual(described, ['waiting', 'nothing yet', 'got 4', 'refused: storage']);
});
