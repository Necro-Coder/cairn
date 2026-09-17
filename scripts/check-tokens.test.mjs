/**
 * Tests for the design token gate.
 *
 * A gate nobody has seen fail is a gate nobody knows works. Each test here is one of the
 * things the gate claims to catch, written the way somebody would actually write the
 * mistake, plus the cases it must not catch — which are the ones that decide whether
 * people keep it switched on.
 *
 * Run by `node --test scripts/`, which needs no test framework: Node runs TypeScript and
 * carries a test runner, so the dependency count stays where it is.
 */

import { strict as assert } from 'node:assert';
import { test } from 'node:test';

import { findViolations } from './check-tokens.mjs';

/** The rule names of everything found, which is what each test is actually asserting. */
const rulesIn = (source) => findViolations(source).map((violation) => violation.rule);

test('a declaration built from tokens is accepted', () => {
  assert.deepEqual(
    rulesIn(`.card {
      padding: var(--space-5);
      border: var(--border-width) solid var(--colour-border);
      border-radius: var(--radius-md);
      background-color: var(--colour-surface-raised);
      transition: opacity var(--duration-fast) var(--easing);
    }`),
    [],
  );
});

test('a colour literal is rejected, in every notation', () => {
  assert.deepEqual(rulesIn('a { color: #ff5a1f; }'), ['colour']);
  assert.deepEqual(rulesIn('a { color: #fff; }'), ['colour']);
  assert.deepEqual(rulesIn('a { background: rgba(20, 19, 15, 0.42); }'), ['colour']);
  assert.deepEqual(rulesIn('a { background: hsl(20 90% 40%); }'), ['colour']);
  assert.deepEqual(rulesIn('a { background: oklch(0.6 0.2 40); }'), ['colour']);
});

test('a length is rejected whether or not it is on the scale', () => {
  // 16px is exactly --space-4. It is still a defect: a component that writes the number
  // has stopped reading the tokens, and the next one it writes will not be on the scale.
  assert.deepEqual(rulesIn('a { padding: 16px; }'), ['length']);
  assert.deepEqual(rulesIn('a { padding: 13px; }'), ['length']);
  assert.deepEqual(rulesIn('a { max-width: 34rem; }'), ['length']);
  assert.deepEqual(rulesIn('a { margin: 0 0 1.5rem; }'), ['length']);
});

test('a duration written by hand is rejected', () => {
  assert.deepEqual(rulesIn('a { transition: opacity 160ms ease; }'), ['duration']);
  assert.deepEqual(rulesIn('a { animation-duration: 0.3s; }'), ['duration']);
});

test('a relative unit is not a hand-written value and is left alone', () => {
  // Each of these says "the same as something this element already has", which is not a
  // value typed in from nowhere. Policing them would push people towards absolute units.
  assert.deepEqual(rulesIn('a { letter-spacing: 0.09em; font-size: 0.925em; }'), []);
  assert.deepEqual(rulesIn('a { max-width: 62ch; width: 50%; height: 100dvh; }'), []);
  assert.deepEqual(rulesIn('a { grid-template-columns: 1fr 2fr; }'), []);
  assert.deepEqual(rulesIn('a { width: 0; border: 0; }'), []);
});

test('a media query may carry a length, because it cannot read a token', () => {
  assert.deepEqual(
    rulesIn(`@media (max-width: 880px) {
      .marks { display: none; }
    }`),
    [],
  );
});

test('a length inside a media query block is still rejected', () => {
  // Only the prelude is exempt. A rule inside it is an ordinary rule.
  assert.deepEqual(
    rulesIn(`@media (max-width: 420px) {
      .marks { padding: 12px; }
    }`),
    ['length'],
  );
});

test('a comment may name a value without being reported for it', () => {
  // The comment explaining why 880 is the breakpoint is the most useful line in the file.
  // Reporting it would teach people to delete the explanation.
  assert.deepEqual(rulesIn('/* 880px is the window minimum: #ff5a1f is the mark. */'), []);
  assert.deepEqual(rulesIn('<!-- the composition goes at 160ms, or #14130f -->'), []);
});

test('a declared exemption lets exactly one line through', () => {
  assert.deepEqual(
    rulesIn(`.thing {
      /* tokens-exempt: the platform draws this control and only accepts a pixel value */
      scrollbar-width: 8px;
      padding: 12px;
    }`),
    ['length'],
    'the exemption covered the line after the one it was written for',
  );
});

test('an exemption with no reason is not an exemption', () => {
  assert.deepEqual(
    rulesIn(`.thing {
      /* tokens-exempt: */
      padding: 12px;
    }`),
    ['length'],
  );
});

test('a violation is reported where it is, so it can be found', () => {
  const [violation] = findViolations('.a {\n  color: #14130f;\n}', 'src/x.css');

  assert.equal(violation?.path, 'src/x.css');
  assert.equal(violation?.line, 2);
  assert.equal(violation?.text, '#14130f');
  assert.match(violation?.advice ?? '', /--colour-/);
});

test('every violation in a file is reported, not only the first', () => {
  assert.deepEqual(rulesIn('.a { color: #fff; padding: 4px; transition: all 90ms; }'), [
    'colour',
    'length',
    'duration',
  ]);
});

test('a component is scanned outside its style block as well', () => {
  // An inline style attribute is the obvious way around a gate that only read `<style>`,
  // and it is now caught twice: once for the colour and once for the attribute itself.
  assert.deepEqual(rulesIn('<span style="background: #f2c007"></span>'), [
    'colour',
    'inline-style',
  ]);
});

test('the numbers an icon is drawn with are not lengths', () => {
  // Every icon in this application is a 24 box with a 1.5 stroke, and none of those
  // numbers carries a unit. A gate that flagged them would be switched off within a day.
  assert.deepEqual(
    rulesIn(
      '<svg viewBox="0 0 24 24" width="24" stroke-width="1.5" stroke="currentColor">' +
        '<path d="M12 5v14M5 12h14" /></svg>',
    ),
    [],
  );
});

test('an inline style attribute is rejected, because the policy drops it in the real window', () => {
  // Both rules fire: the attribute is the defect, and the colour inside it is a second one.
  const found = findViolations('<div style="--tone: #ff0000"></div>', 'a.svelte');

  assert.deepEqual(
    found.map((violation) => violation.rule),
    ['colour', 'inline-style'],
  );
});

test('a style attribute given a Svelte expression is rejected too', () => {
  const found = findViolations('<div style={whatever}></div>', 'a.svelte');

  assert.deepEqual(
    found.map((violation) => violation.rule),
    ['inline-style'],
  );
});

test('the word style in ordinary markup is not an inline style', () => {
  assert.deepEqual(findViolations('<p>lifestyle=</p>\n<p class="style">x</p>', 'a.svelte'), []);
});
