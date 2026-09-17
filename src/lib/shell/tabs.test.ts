/**
 * The tab strip's rules, checked without a window.
 *
 * Run by `node --test`, which strips the types and executes the module directly. That is
 * why `tabs.ts` holds no runes and imports `SectionId` as a type: everything that survives
 * to runtime here is plain JavaScript over plain objects.
 *
 * The cases are the ones written into the phase plan — born temporary, replacing the
 * temporary one, pinning with a double click, not releasing one already pinned, the cap of
 * six and of four, and reopening the last closed one pinned — plus the edges around them
 * that would otherwise only be found by somebody clicking.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  MAX_TABS,
  MAX_TABS_NARROW,
  activate,
  activeTab,
  capacityAt,
  close,
  cycle,
  initial,
  isClosable,
  move,
  open,
  pin,
  reopen,
  type TabState,
} from './tabs.ts';

/** The cap a full-width window gives, which is what most of these run under. */
const WIDE = { capacity: MAX_TABS };

/** Opening something for a look, which is what the menu, a shortcut and the palette do. */
const GLANCE = { temporary: true, capacity: MAX_TABS };

/** What the strip is showing, as section names, which is what a person would read off it. */
function sections(state: TabState): string[] {
  return state.tabs.map((tab) => tab.section);
}

test('the strip starts as the panel alone, and the panel cannot be closed', () => {
  const state = initial();

  assert.deepEqual(sections(state), ['panel']);
  assert.equal(activeTab(state).section, 'panel');
  assert.equal(activeTab(state).temporary, false);
  assert.equal(isClosable(activeTab(state)), false);
  assert.deepEqual(close(state, state.activeId), state);
});

test('anything opened is born temporary and is the one on screen', () => {
  const state = open(initial(), 'habits', GLANCE);

  assert.deepEqual(sections(state), ['panel', 'habits']);
  assert.equal(activeTab(state).section, 'habits');
  assert.equal(activeTab(state).temporary, true);
});

test('opening a second thing replaces the temporary one instead of adding beside it', () => {
  const state = open(open(initial(), 'habits', GLANCE), 'passwords', GLANCE);

  assert.deepEqual(sections(state), ['panel', 'passwords']);
  assert.equal(activeTab(state).section, 'passwords');
});

test('a double click makes the temporary tab permanent, and the next thing opened no longer replaces it', () => {
  const glanced = open(initial(), 'habits', GLANCE);
  const pinned = pin(glanced, glanced.activeId);

  assert.equal(pinned.tabs[1]?.temporary, false);

  const next = open(pinned, 'passwords', GLANCE);
  assert.deepEqual(sections(next), ['panel', 'habits', 'passwords']);
});

test('a double click on a tab that is already permanent leaves it permanent', () => {
  const glanced = open(initial(), 'habits', GLANCE);
  const once = pin(glanced, glanced.activeId);
  const twice = pin(once, glanced.activeId);

  assert.equal(twice.tabs[1]?.temporary, false);
  assert.deepEqual(twice, once);
});

test('opening something already open goes to it rather than opening it twice', () => {
  const two = open(pin(open(initial(), 'habits', GLANCE), 't1'), 'passwords', GLANCE);
  const back = open(two, 'habits', GLANCE);

  assert.deepEqual(sections(back), ['panel', 'habits', 'passwords']);
  assert.equal(activeTab(back).section, 'habits');
});

test('reaching a tab deliberately makes it permanent, even when it was only being glanced at', () => {
  const glanced = open(initial(), 'habits', GLANCE);
  const deliberate = open(glanced, 'habits', { temporary: false, ...WIDE });

  assert.equal(deliberate.tabs[1]?.temporary, false);
});

test('the seventh thing opened recycles the temporary tab instead of shrinking the strip', () => {
  // Six tabs besides the panel cannot be reached from today's menu: there are five sections
  // and opening one that is already open goes to it. The strip only fills up once 02.1b
  // gives a section more than one tab, so the state is built by hand, with the repeats that
  // will then be ordinary.
  const full: TabState = {
    tabs: [
      { id: 't0', section: 'panel', temporary: false },
      { id: 't1', section: 'habits', temporary: false },
      { id: 't2', section: 'habits', temporary: false },
      { id: 't3', section: 'passwords', temporary: false },
      { id: 't4', section: 'passwords', temporary: false },
      { id: 't5', section: 'settings', temporary: false },
      { id: 't6', section: 'settings', temporary: true },
    ],
    activeId: 't6',
    closed: [],
    nextId: 7,
  };
  assert.equal(full.tabs.length, MAX_TABS + 1);

  const seventh = open(full, 'finances', GLANCE);
  assert.equal(seventh.tabs.length, MAX_TABS + 1);
  assert.equal(seventh.tabs.at(-1)?.section, 'finances');
  assert.equal(seventh.tabs.at(-1)?.temporary, true);
});

test('with the strip full and nothing being glanced at, the last tab is what gives way', () => {
  const full: TabState = {
    tabs: [
      { id: 't0', section: 'panel', temporary: false },
      { id: 't1', section: 'habits', temporary: false },
      { id: 't2', section: 'habits', temporary: false },
      { id: 't3', section: 'passwords', temporary: false },
      { id: 't4', section: 'passwords', temporary: false },
      { id: 't5', section: 'settings', temporary: false },
      { id: 't6', section: 'settings', temporary: false },
    ],
    activeId: 't0',
    closed: [],
    nextId: 7,
  };

  const opened = open(full, 'finances', GLANCE);
  assert.equal(opened.tabs.length, MAX_TABS + 1);
  assert.equal(opened.tabs.at(-1)?.section, 'finances');
  assert.equal(activeTab(opened).section, 'finances');
});

test('a narrow window caps the strip at four, and a wide one at six', () => {
  assert.equal(capacityAt(879), MAX_TABS_NARROW);
  assert.equal(capacityAt(880), MAX_TABS);
  assert.equal(capacityAt(1440), MAX_TABS);
});

test('narrowing the window does not close tabs that are already open', () => {
  let state: TabState = initial();
  for (const section of ['habits', 'passwords', 'finances', 'settings'] as const) {
    state = open(state, section, GLANCE);
    state = pin(state, state.activeId);
  }
  assert.equal(state.tabs.length, MAX_TABS_NARROW + 1);

  // The cap governs opening and nothing else. There is no function that takes a width and
  // returns a shorter strip, and this asserts that: opening under the narrow cap reuses a
  // slot, and everything that was there is still there.
  const narrow = open(state, 'habits', { temporary: true, capacity: MAX_TABS_NARROW });
  assert.equal(narrow.tabs.length, MAX_TABS_NARROW + 1);
  assert.deepEqual(sections(narrow), ['panel', 'habits', 'passwords', 'finances', 'settings']);
});

test('closing the tab on screen moves to the one that took its place', () => {
  let state: TabState = initial();
  for (const section of ['habits', 'passwords', 'finances'] as const) {
    state = open(state, section, GLANCE);
    state = pin(state, state.activeId);
  }
  const middle = state.tabs[2];
  assert.ok(middle !== undefined);

  const closed = close(activate(state, middle.id), middle.id);
  assert.deepEqual(sections(closed), ['panel', 'habits', 'finances']);
  assert.equal(activeTab(closed).section, 'finances');
});

test('closing the last tab moves to the one before it', () => {
  const state = pin(open(initial(), 'habits', GLANCE), 't1');
  const closed = close(state, 't1');

  assert.deepEqual(sections(closed), ['panel']);
  assert.equal(activeTab(closed).section, 'panel');
});

test('closing a tab that is not on screen leaves the screen alone', () => {
  let state: TabState = initial();
  state = pin(open(state, 'habits', GLANCE), 't1');
  state = pin(open(state, 'passwords', GLANCE), 't2');
  state = activate(state, 't1');

  const closed = close(state, 't2');
  assert.equal(activeTab(closed).section, 'habits');
});

test('the last closed tab comes back permanent, and only once', () => {
  const state = close(pin(open(initial(), 'passwords', GLANCE), 't1'), 't1');
  assert.deepEqual(state.closed, ['passwords']);

  const back = reopen(state, MAX_TABS);
  assert.deepEqual(sections(back), ['panel', 'passwords']);
  assert.equal(back.tabs[1]?.temporary, false);
  assert.equal(activeTab(back).section, 'passwords');
  assert.deepEqual(back.closed, []);

  assert.deepEqual(reopen(back, MAX_TABS), back);
});

test('reopening does not take over a tab that is being glanced at', () => {
  const closed = close(pin(open(initial(), 'passwords', GLANCE), 't1'), 't1');
  const glancing = open(closed, 'habits', GLANCE);

  const back = reopen(glancing, MAX_TABS);
  assert.deepEqual(sections(back), ['panel', 'habits', 'passwords']);
});

test('going to a tab that is not there changes nothing', () => {
  const state = open(initial(), 'habits', GLANCE);

  assert.deepEqual(activate(state, 'nothing'), state);
});

test('a tab can be dragged elsewhere in the strip, but never in front of the panel', () => {
  let state: TabState = initial();
  for (const section of ['habits', 'passwords', 'finances'] as const) {
    state = open(state, section, GLANCE);
    state = pin(state, state.activeId);
  }

  assert.deepEqual(sections(move(state, 't3', 1)), ['panel', 'finances', 'habits', 'passwords']);
  assert.deepEqual(move(state, 't3', 0), state);
  assert.deepEqual(move(state, 't0', 2), state);
  assert.deepEqual(move(state, 't3', 9), state);
  assert.deepEqual(move(state, 'nothing', 1), state);
});

test('cycling walks the strip and wraps at both ends', () => {
  let state: TabState = initial();
  for (const section of ['habits', 'passwords'] as const) {
    state = open(state, section, GLANCE);
    state = pin(state, state.activeId);
  }
  state = activate(state, 't0');

  assert.equal(activeTab(cycle(state, 1)).section, 'habits');
  assert.equal(activeTab(cycle(state, -1)).section, 'passwords');
  assert.equal(activeTab(cycle(cycle(cycle(state, 1), 1), 1)).section, 'panel');
});
