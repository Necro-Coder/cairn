/**
 * The rules of the tab strip, as functions over values.
 *
 * Nothing here knows about Svelte, about the window, or about what a tab draws. It is the
 * state machine, and it is separate from the reactive wrapper in `workspace.svelte.ts` for
 * one reason: these rules are the part of the navigation that is easy to get subtly wrong, and
 * a test over plain values is the only way to be sure they are right without driving a
 * browser through nine steps to check one of them.
 *
 * The rules, in the order they matter.
 *
 * **The panel is first and cannot be closed.** It is what unlocking opens onto.
 *
 * **Everything opened is born temporary**, whether from the menu, from the palette or from
 * a shortcut, and there is exactly one temporary slot. The next thing opened replaces what
 * is in it rather than accumulating beside it. That is what stops somebody ending up with
 * twenty tabs they never decided to have.
 *
 * **A temporary tab becomes permanent** on a double click. Double-clicking one that is
 * already permanent does nothing: letting it go back to temporary would be an invisible
 * change of state that later loses the tab on its own.
 *
 * **The strip never scrolls.** There is a cap, and past it opening something recycles the
 * temporary slot instead of shrinking what is already there. A browser's twentieth tab is
 * eight pixels wide and unreadable, and nothing tells you when that happened.
 *
 * **Reopening the last closed tab brings it back permanent.** It is the one exception to
 * everything being born temporary, because it was asked for deliberately.
 *
 * Every function returns a new state. Nothing is mutated, so a caller cannot half apply a
 * change, and the reactive wrapper's single assignment is what the interface reacts to.
 */

import type { SectionId } from './sections';

/** The section the strip always starts with, and the only one that cannot be closed. */
const PINNED_FIRST: SectionId = 'panel';

/**
 * How many tabs besides the panel fit while still being readable.
 *
 * Six, and four when the window is below 880 pixels. Both are decisions rather than
 * measurements: a strip that always reads whole is worth more than a strip that holds
 * everything.
 */
export const MAX_TABS = 6;

/** The same, for a window narrower than the layout's own minimum width. */
export const MAX_TABS_NARROW = 4;

/** One tab. */
export interface Tab {
  /** Stable for the life of the tab, so that reordering and closing cannot confuse two. */
  readonly id: string;
  /** What it shows. */
  readonly section: SectionId;
  /** Whether it is the one slot that the next thing opened will take over. */
  readonly temporary: boolean;
}

/** Everything the strip knows. */
export interface TabState {
  readonly tabs: readonly Tab[];
  /** Which one is on screen. Always the id of a tab that is in the list. */
  readonly activeId: string;
  /**
   * The sections of tabs that were closed, most recent last.
   *
   * Sections rather than tabs: a reopened tab is a new tab, and giving it the old one's
   * identity would mean two tabs could end up sharing an id after a close and a reopen.
   */
  readonly closed: readonly SectionId[];
  /** What the next tab's id is built from. */
  readonly nextId: number;
}

/** The strip as it is the moment the vault opens: the panel, and nothing else. */
export function initial(): TabState {
  return {
    tabs: [{ id: 't0', section: PINNED_FIRST, temporary: false }],
    activeId: 't0',
    closed: [],
    nextId: 1,
  };
}

/** Whether a tab may be closed. The panel may not. */
export function isClosable(tab: Tab): boolean {
  return tab.section !== PINNED_FIRST;
}

/** The tab on screen, or the panel if the active id somehow names nothing. */
export function activeTab(state: TabState): Tab {
  const found = state.tabs.find((tab) => tab.id === state.activeId);
  if (found === undefined) {
    // Unreachable while every function here keeps `activeId` pointing at a tab in the
    // list, and this is what makes the return type honest rather than optional.
    throw new Error('la pestaña activa no está en la lista');
  }
  return found;
}

/** How many tabs may be open at once, for a window of this width. */
export function capacityAt(windowWidth: number): number {
  return windowWidth < 880 ? MAX_TABS_NARROW : MAX_TABS;
}

/**
 * Opens a section.
 *
 * Already open means going to it, and a deliberate open — reopening, or anything else that
 * is not a glance — also makes it permanent. Otherwise it takes the temporary slot, and
 * only when there is neither a slot nor room does a new tab appear.
 *
 * `capacity` counts tabs besides the panel, and is what the caller decides from the width
 * of the window. It governs opening and nothing else: a window narrowed with six tabs
 * already open keeps its six. Closing tabs because somebody dragged a corner would throw
 * away state nobody asked to lose, which is a worse outcome than a strip that is briefly
 * tighter than it was designed to be.
 */
export function open(
  state: TabState,
  section: SectionId,
  options: { readonly temporary: boolean; readonly capacity: number },
): TabState {
  const existing = state.tabs.find((tab) => tab.section === section);
  if (existing !== undefined) {
    return {
      ...state,
      activeId: existing.id,
      tabs: options.temporary
        ? state.tabs
        : state.tabs.map((tab) => (tab.id === existing.id ? { ...tab, temporary: false } : tab)),
    };
  }

  const slot = state.tabs.find((tab) => tab.temporary);
  const full = state.tabs.length - 1 >= options.capacity;

  // A glance takes the slot; a deliberate open does not, because taking it would throw
  // away whatever was being glanced at in order to open something else.
  //
  // Past the cap everything takes a slot, and where there is none it is the last tab that
  // gives way. Something has to: the strip does not scroll, and a menu entry that did
  // nothing would read as a broken application. The last tab is the one furthest from the
  // panel and the one whose disappearance moves nothing else.
  let recycled: Tab | undefined = options.temporary ? slot : undefined;
  if (full) {
    recycled = slot ?? state.tabs.at(-1);
  }

  if (recycled !== undefined && recycled.section !== PINNED_FIRST) {
    return {
      ...state,
      activeId: recycled.id,
      tabs: state.tabs.map((tab) =>
        tab.id === recycled.id ? { ...tab, section, temporary: options.temporary } : tab,
      ),
    };
  }

  const id = `t${String(state.nextId)}`;

  return {
    ...state,
    tabs: [...state.tabs, { id, section, temporary: options.temporary }],
    activeId: id,
    nextId: state.nextId + 1,
  };
}

/**
 * Makes a tab permanent.
 *
 * A tab that is already permanent is left alone. Letting a second double click release it
 * would be a change of state with nothing on screen to announce it, and the tab would then
 * disappear on its own the next time anything was opened.
 */
export function pin(state: TabState, id: string): TabState {
  return {
    ...state,
    tabs: state.tabs.map((tab) => (tab.id === id ? { ...tab, temporary: false } : tab)),
  };
}

/** Goes to a tab. Going to one that is not there changes nothing. */
export function activate(state: TabState, id: string): TabState {
  return state.tabs.some((tab) => tab.id === id) ? { ...state, activeId: id } : state;
}

/**
 * Closes a tab.
 *
 * The panel cannot be closed. Closing the one on screen moves to whatever took its place,
 * or to the one before it if it was the last, which is where the eye already is.
 */
export function close(state: TabState, id: string): TabState {
  const index = state.tabs.findIndex((tab) => tab.id === id);
  const tab = state.tabs[index];
  if (tab === undefined || !isClosable(tab)) {
    return state;
  }

  const tabs = state.tabs.filter((each) => each.id !== id);
  const neighbour = tabs[Math.min(index, tabs.length - 1)];

  return {
    ...state,
    tabs,
    activeId:
      state.activeId === id ? (neighbour?.id ?? tabs[0]?.id ?? state.activeId) : state.activeId,
    closed: [...state.closed, tab.section],
  };
}

/**
 * Brings back the last tab that was closed, permanent.
 *
 * The one exception to everything being born temporary. Somebody who asks for a tab back
 * has decided they want it, which is exactly what a permanent tab means.
 */
export function reopen(state: TabState, capacity: number): TabState {
  const section = state.closed.at(-1);
  if (section === undefined) {
    return state;
  }

  return open({ ...state, closed: state.closed.slice(0, -1) }, section, {
    temporary: false,
    capacity,
  });
}

/** Moves a tab to another place in the strip, which is what dragging one does. */
export function move(state: TabState, id: string, toIndex: number): TabState {
  const from = state.tabs.findIndex((tab) => tab.id === id);
  const tab = state.tabs[from];
  // The panel stays first, and nothing may be dropped in front of it: it is the one tab
  // whose position is part of what it is.
  if (tab === undefined || from < 1 || toIndex < 1 || toIndex >= state.tabs.length) {
    return state;
  }

  const tabs = [...state.tabs];
  tabs.splice(from, 1);
  tabs.splice(toIndex, 0, tab);

  return { ...state, tabs };
}

/** Goes to the next tab, wrapping, which is what cycling through them does. */
export function cycle(state: TabState, step: number): TabState {
  const here = state.tabs.findIndex((tab) => tab.id === state.activeId);
  const count = state.tabs.length;
  const next = state.tabs[(((here + step) % count) + count) % count];

  return next === undefined ? state : { ...state, activeId: next.id };
}
