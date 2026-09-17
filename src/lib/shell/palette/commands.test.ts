/**
 * How the palette picks a command, checked without a window.
 *
 * The cases that matter are the Spanish ones. An interface written with accents, searched by
 * somebody who does not type them, is the difference between a palette that gets used and a
 * palette that gets abandoned on the second day.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { fold, rank, score, type Command } from './commands.ts';

/** A catalogue small enough to reason about, built here rather than read from the real one. */
const CLOSE_TAB: Command = {
  id: 'tab:close',
  title: 'Cerrar la pestaña actual',
  detail: 'Cierra la pestaña que estás viendo',
  group: 'Pestañas',
  shortcut: 'Ctrl W',
  effect: { kind: 'closeTab' },
};

const COMMANDS: readonly Command[] = [
  {
    id: 'open:habits',
    title: 'Hábitos',
    detail: 'Abre hábitos',
    group: 'Ir a',
    shortcut: 'Ctrl 1',
    effect: { kind: 'open', section: 'habits' },
  },
  {
    id: 'open:passwords',
    title: 'Contraseñas',
    detail: 'Abre contraseñas',
    group: 'Ir a',
    shortcut: 'Ctrl 2',
    effect: { kind: 'open', section: 'passwords' },
  },
  CLOSE_TAB,
  {
    id: 'vault:lock',
    title: 'Cerrar la caja fuerte',
    detail: 'Bloquea la aplicación ahora',
    group: 'Caja fuerte',
    shortcut: 'Ctrl L',
    effect: { kind: 'lock' },
  },
];

/** What the palette would show, as ids, which is what a person would see in order. */
function shown(query: string, history: readonly string[] = []): string[] {
  return rank(COMMANDS, query, history).map((command) => command.id);
}

test('accents and case are folded away before anything is compared', () => {
  assert.equal(fold('Hábitos'), 'habitos');
  assert.equal(fold('  Contraseñas  '), 'contrasenas');
  assert.equal(fold('ÁÉÍÓÚÜ'), 'aeiouu');
});

test('a query without accents finds the entry that has them', () => {
  assert.deepEqual(shown('habitos'), ['open:habits']);
  assert.deepEqual(shown('contrasena'), ['open:passwords']);
});

test('a query with accents still finds it', () => {
  assert.deepEqual(shown('hábitos'), ['open:habits']);
});

test('a title that starts with what was typed comes before one that merely contains it', () => {
  assert.deepEqual(shown('cerrar'), ['tab:close', 'vault:lock']);
  assert.equal(score(CLOSE_TAB, 'cerrar'), 3);
  assert.equal(score(CLOSE_TAB, 'pestaña'), 2);
  assert.equal(score(CLOSE_TAB, 'viendo'), 1);
  assert.equal(score(CLOSE_TAB, 'nada de esto'), null);
});

test('a query nothing answers shows nothing, rather than showing everything', () => {
  assert.deepEqual(shown('zzz'), []);
});

test('with nothing typed, what was run recently is at the top', () => {
  assert.deepEqual(shown(''), ['open:habits', 'open:passwords', 'tab:close', 'vault:lock']);

  assert.deepEqual(shown('', ['vault:lock', 'tab:close']), [
    'vault:lock',
    'tab:close',
    'open:habits',
    'open:passwords',
  ]);
});

test('a history entry that no longer exists is ignored rather than drawn as a blank row', () => {
  assert.deepEqual(shown('', ['open:gone', 'vault:lock']), [
    'vault:lock',
    'open:habits',
    'open:passwords',
    'tab:close',
  ]);
});

test('once something is typed the history stops deciding the order', () => {
  assert.deepEqual(shown('cerrar', ['vault:lock']), ['tab:close', 'vault:lock']);
});

test('a query of nothing but spaces is the same as no query at all', () => {
  assert.deepEqual(shown('   '), shown(''));
});
