/**
 * Which section is on screen.
 *
 * The smallest thing that can be called a router: one value, and a way to change it. The
 * tab strip replaces it in the next step of this phase, and it exists now so that the tab
 * strip replaces something that already worked rather than appearing over a hole.
 *
 * It holds nothing derived from an open vault — a section name is the same handful of
 * words printed in the menu — so there is nothing here for the lock to discard.
 */

import type { SectionId } from './sections';

/** The section somebody sees on opening the vault. */
const FIRST: SectionId = 'panel';

/** Where in the application the window currently is. */
class Router {
  /** The section on screen. */
  current = $state<SectionId>(FIRST);

  /** Goes somewhere. Going where you already are is not a change and costs nothing. */
  open(id: SectionId): void {
    this.current = id;
  }

  /** Returns to the section an unlock starts on. Called when the vault closes. */
  reset(): void {
    this.current = FIRST;
  }
}

/** The one router the whole interface shares. */
export const router = new Router();
