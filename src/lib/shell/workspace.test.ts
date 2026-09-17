/**
 * What a lock throws away, checked without a window.
 *
 * The case that matters is the last one. It builds a workspace that looks like a working
 * afternoon — tabs open, cards on the panel, a palette history — and asserts that closing
 * the vault leaves nothing of any of it. That is the phase plan's manual test 12 written
 * down where it cannot be forgotten, and it is the regression test for the defect in PR #31,
 * where a panel that was a variable of its own survived a lock because nobody cleared it.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { open as openTab, pin } from './tabs.ts';
import {
  MAX_HISTORY,
  emptyWorkspace,
  forVault,
  remembered,
  withCard,
  withSettingsSection,
  withTabs,
  withoutCard,
  type Workspace,
} from './workspace.ts';

/** A workspace that looks like somebody has been using the application for an hour. */
function busy(): Workspace {
  let workspace = emptyWorkspace();

  for (const section of ['habits', 'passwords', 'finances'] as const) {
    const tabs = openTab(workspace.tabs, section, { temporary: true, capacity: 6 });
    workspace = withTabs(workspace, pin(tabs, tabs.activeId));
  }

  workspace = withCard(workspace, 'finances-month');
  workspace = withCard(workspace, 'passwords-recent');
  workspace = remembered(workspace, 'open:passwords');
  workspace = withSettingsSection(workspace, 'diagnostics');

  return workspace;
}

test('a workspace opens with one tab, no cards and no history', () => {
  const workspace = emptyWorkspace();

  assert.equal(workspace.tabs.tabs.length, 1);
  assert.deepEqual(workspace.cards, []);
  assert.deepEqual(workspace.history, []);
  assert.equal(workspace.settings, 'security');
});

test('an open vault keeps the workspace it already had', () => {
  const workspace = busy();

  assert.equal(forVault(workspace, true), workspace);
});

test('unlocking with nothing behind it starts a fresh workspace', () => {
  assert.deepEqual(forVault(null, true), emptyWorkspace());
});

test('a card goes on the panel once, and comes off again', () => {
  const once = withCard(emptyWorkspace(), 'habits-today');
  assert.deepEqual(once.cards, ['habits-today']);

  const twice = withCard(once, 'habits-today');
  assert.equal(twice, once);

  const both = withCard(once, 'finances-month');
  assert.deepEqual(both.cards, ['habits-today', 'finances-month']);

  assert.deepEqual(withoutCard(both, 'habits-today').cards, ['finances-month']);
  assert.equal(withoutCard(both, 'passwords-review'), both);
});

test('the palette remembers the last few things run, most recent first and without repeats', () => {
  let workspace = emptyWorkspace();
  for (const entry of ['a', 'b', 'c']) {
    workspace = remembered(workspace, entry);
  }
  assert.deepEqual(workspace.history, ['c', 'b', 'a']);

  workspace = remembered(workspace, 'a');
  assert.deepEqual(workspace.history, ['a', 'c', 'b']);

  for (let n = 0; n < MAX_HISTORY * 2; n += 1) {
    workspace = remembered(workspace, `entry-${String(n)}`);
  }
  assert.equal(workspace.history.length, MAX_HISTORY);
  assert.equal(workspace.history[0], `entry-${String(MAX_HISTORY * 2 - 1)}`);
});

test('closing the vault discards the tabs, the cards and the history, all of them', () => {
  const workspace = busy();

  // Everything the assertion below is about was really there first. A regression test that
  // asserts nothing survives is worth nothing if there was nothing to survive.
  assert.equal(workspace.tabs.tabs.length, 4);
  assert.equal(workspace.cards.length, 2);
  assert.equal(workspace.history.length, 1);
  assert.equal(workspace.settings, 'diagnostics');

  assert.equal(forVault(workspace, false), null);

  // And opening it again starts from nothing rather than from what was there, which is what
  // manual test 12 looks for on screen: no trace in the strip and none in the palette.
  assert.deepEqual(forVault(forVault(workspace, false), true), emptyWorkspace());
});
