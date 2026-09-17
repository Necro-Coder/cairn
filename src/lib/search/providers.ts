/**
 * The three search providers, as they are before there is anything to search.
 *
 * Each module registers one. Today all three answer with nothing, and the palette says so
 * with the badge every unwritten part of the application carries, because there is no data
 * yet and there is deliberately no fixture standing in for it: a fake index is code that
 * gets thrown away, and a fake index of passwords is a fake index of passwords.
 *
 * What is real here is the shape. Phase 03 replaces each `search` with a call to a
 * `search_titles` command in the core, over titles decrypted into memory at unlock, and
 * nothing outside this file changes — which is the point of having written the contract
 * before the data existed.
 */

import type { ModuleId, SearchProvider } from './contract';

/** Which modules will be searchable, in the order their results are grouped. */
const MODULES: readonly ModuleId[] = ['habits', 'passwords', 'finances'];

/** What the palette shows where the results will go, until there are any. */
export const SEARCH_NOTICE = 'La búsqueda llega con los módulos. Todavía no hay nada que buscar.';

/**
 * Whether any provider can actually answer.
 *
 * A flag rather than "the list came back empty", because those are different things and the
 * interface has to say different things about them. An empty answer from a working search
 * means nothing matched; an empty answer from this one means the search is not written yet,
 * and telling somebody "no results" would be a lie about their own data.
 */
// Widened to `boolean` on purpose: as the literal `false` the compiler would narrow every
// branch that asks about it to unreachable code, and the day this becomes true the diff
// should be one word rather than one word and whatever the narrowing hid.
export const SEARCH_IS_READY: boolean = false;

/** The three, all of them empty. */
export const PROVIDERS: readonly SearchProvider[] = MODULES.map((module) => ({
  module,
  // eslint-disable-next-line @typescript-eslint/require-await -- the shape is the contract's, and phase 03 fills it with a call to the core
  async search() {
    return [];
  },
}));
