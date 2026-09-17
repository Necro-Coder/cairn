/**
 * Everything the interface derives from an open vault, and the rules for changing it.
 *
 * Three things live here: which tabs are open, which cards are on the panel, and what has
 * recently been run from the palette. They are one object rather than three variables for
 * one reason, and it is the reason this file exists at all: **when the vault closes, all of
 * it is discarded in a single assignment**, and a single assignment cannot half happen.
 *
 * The defect the reactive wrapper is written against is a real one. In PR #31 the
 * diagnostics panel survived a lock, because it was a variable of its own that nobody
 * remembered to clear, and the lock screen appeared underneath a screen somebody had walked
 * away from. Anything derived from an open vault that is added later goes in here, and is
 * discarded by the line that already exists rather than by a line somebody has to think to
 * write.
 *
 * Nothing survives a lock. Not the tabs, not the cards, not the history — the phase plan's
 * manual test is that after locking and unlocking, neither the strip nor the palette shows
 * a trace of what was open. Where these are kept between runs of the application, in phase
 * 03, it will be in the encrypted vault and behind the same unlock.
 *
 * The rules are functions over values so that `node --test` can check them. The runes are
 * in `workspace.svelte.ts`, which is a wrapper over this and holds no rules of its own.
 */

import type { CardId } from './panel/cards';
import { initial, type TabState } from './tabs.ts';

/**
 * How many entries the palette remembers.
 *
 * Eight, which is one short screenful. The list exists so that the thing somebody does
 * twenty times a day is one keystroke away, and that is satisfied by the last few; a longer
 * list would push the search results off the screen to hold entries nobody rereads.
 */
export const MAX_HISTORY = 8;

/** Everything that came from an open vault. */
export interface Workspace {
  /** The tab strip. */
  readonly tabs: TabState;
  /** What is on the panel, in the order it was put there. */
  readonly cards: readonly CardId[];
  /** What was last run from the palette, most recent first. */
  readonly history: readonly string[];
}

/** A workspace as it is the moment the vault opens: one tab, no cards, no history. */
export function emptyWorkspace(): Workspace {
  return { tabs: initial(), cards: [], history: [] };
}

/**
 * The workspace that belongs to a given state of the vault.
 *
 * The whole rule, in one place: an open vault has a workspace, and a closed one has none.
 * Written as a function rather than as two lines inside the store so that the test can hand
 * it a workspace with six tabs, a full panel and a history, and assert that closing the
 * vault leaves nothing of it.
 */
export function forVault(open: Workspace | null, unlocked: boolean): Workspace | null {
  if (!unlocked) {
    return null;
  }
  return open ?? emptyWorkspace();
}

/**
 * Puts a card on the panel.
 *
 * At the end, because that is where the eye last was, and only once: a card is a view of a
 * module and two copies of it would show the same thing twice.
 */
export function withCard(workspace: Workspace, id: CardId): Workspace {
  if (workspace.cards.includes(id)) {
    return workspace;
  }
  return { ...workspace, cards: [...workspace.cards, id] };
}

/** Takes a card off the panel. Taking off one that is not there changes nothing. */
export function withoutCard(workspace: Workspace, id: CardId): Workspace {
  if (!workspace.cards.includes(id)) {
    return workspace;
  }
  return { ...workspace, cards: workspace.cards.filter((card) => card !== id) };
}

/**
 * Records that something was run from the palette.
 *
 * Most recent first, without repeats, capped. Running something already in the list moves
 * it to the front rather than adding a second copy, which is what makes the list read as
 * "what you do" instead of "what you did".
 */
export function remembered(workspace: Workspace, entry: string): Workspace {
  const history = [entry, ...workspace.history.filter((each) => each !== entry)].slice(
    0,
    MAX_HISTORY,
  );

  return { ...workspace, history };
}

/** Replaces the tab strip, which is how every tab action reaches the workspace. */
export function withTabs(workspace: Workspace, tabs: TabState): Workspace {
  return { ...workspace, tabs };
}
