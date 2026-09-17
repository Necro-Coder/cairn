/**
 * The workspace, as the interface reads and changes it.
 *
 * A wrapper over `workspace.ts` and nothing else: every rule about what a change means
 * lives there, over plain values, where `node --test` can check it. What is here is the one
 * piece of reactive state the shell draws from, and the methods that assign to it.
 *
 * It replaces `router.svelte.ts`, which held a single section name. The shell now has tabs,
 * and a tab strip is a router with more than one answer.
 *
 * Null whenever the vault is closed, and that is the whole point: closing the vault is one
 * assignment that takes the tabs, the panel and the palette history with it.
 */

import type { SectionId } from './sections';
import type { CardId } from './panel/cards';
import { activate, capacityAt, close, cycle, move, open, pin, reopen, type TabState } from './tabs';
import { forVault, remembered, withCard, withTabs, withoutCard, type Workspace } from './workspace';

/**
 * The width assumed before anybody has measured the window.
 *
 * The window minimum, so that the first thing opened before the shell has reported a width
 * is capped as if the window were as small as it is allowed to be. Guessing wide would let
 * a tab open that a narrow window has no room for.
 */
const ASSUMED_WIDTH = 880;

/** The workspace, and the ways it changes. */
class WorkspaceStore {
  /** Everything derived from an open vault, or null while it is closed. */
  state = $state<Workspace | null>(null);

  /** How wide the window is, reported by the shell, which is what the tab cap is read from. */
  width = $state(ASSUMED_WIDTH);

  /** How many tabs besides the panel this window has room for. */
  get capacity(): number {
    return capacityAt(this.width);
  }

  /**
   * Follows the vault.
   *
   * Called from the session on every answer the core gives, rather than from a screen.
   * A screen that has just been unmounted because the vault closed is not somewhere to put
   * the code that clears what the vault left behind — that is the shape of the defect in
   * PR #31, where the thing that should have been cleared was drawn above everything else
   * and therefore never was.
   */
  follow(unlocked: boolean): void {
    this.state = forVault(this.state, unlocked);
  }

  /** Opens a section, born temporary unless somebody asked for it deliberately. */
  open(section: SectionId, options: { readonly temporary: boolean }): void {
    this.#tabs((tabs) => open(tabs, section, { ...options, capacity: this.capacity }));
  }

  /** Goes to a tab. */
  activate(id: string): void {
    this.#tabs((tabs) => activate(tabs, id));
  }

  /** Makes a tab permanent, which is what a double click on it does. */
  pin(id: string): void {
    this.#tabs((tabs) => pin(tabs, id));
  }

  /** Closes a tab. */
  close(id: string): void {
    this.#tabs((tabs) => close(tabs, id));
  }

  /** Brings back the last tab that was closed, permanent. */
  reopen(): void {
    this.#tabs((tabs) => reopen(tabs, this.capacity));
  }

  /** Moves a tab, which is what dragging one does. */
  move(id: string, toIndex: number): void {
    this.#tabs((tabs) => move(tabs, id, toIndex));
  }

  /** Goes to the next tab or the previous one, wrapping. */
  cycle(step: number): void {
    this.#tabs((tabs) => cycle(tabs, step));
  }

  /** Puts a card on the panel. */
  addCard(id: CardId): void {
    this.#edit((workspace) => withCard(workspace, id));
  }

  /** Takes a card off the panel. */
  removeCard(id: CardId): void {
    this.#edit((workspace) => withoutCard(workspace, id));
  }

  /** Records that something was run from the palette. */
  remember(entry: string): void {
    this.#edit((workspace) => remembered(workspace, entry));
  }

  /**
   * Applies a change to the workspace, or does nothing while the vault is closed.
   *
   * The guard is here once rather than in each of the methods above. A change arriving
   * while the vault is closed means a key press or a timer that outlived the screen it came
   * from, and the answer is the same every time: there is no workspace to change.
   */
  #edit(change: (workspace: Workspace) => Workspace): void {
    const here = this.state;
    if (here === null) {
      return;
    }
    this.state = change(here);
  }

  /** The same, for the changes that are about the tab strip. */
  #tabs(change: (tabs: TabState) => TabState): void {
    this.#edit((workspace) => withTabs(workspace, change(workspace.tabs)));
  }
}

/** The one workspace the whole interface shares. */
export const workspace = new WorkspaceStore();
