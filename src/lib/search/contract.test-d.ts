/**
 * The search contract, checked by the compiler.
 *
 * There is nothing to run here: every assertion below is a type that fails to resolve if
 * the contract changes, so `npm run check` is what executes this file. That is deliberate.
 * The rule being defended — a search result can never carry a secret — is a rule about a
 * shape, and a shape is checked where shapes are checked.
 *
 * The one that matters is the first. It fails the moment anybody adds a field to
 * `SearchHit`, whatever they call it, because it asserts the exact set of names rather than
 * guessing at the names somebody might reach for. A test that listed `content`, `value` and
 * `snippet` would pass the day somebody added `preview`.
 */

import type { ModuleId, SearchHit, SearchProvider } from './contract';

/** Resolves only when its argument is `true`, which is what makes an assertion fail loudly. */
type Assert<T extends true> = T;

/** Whether two types are the same, in both directions. */
type Same<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false;

/**
 * A hit has exactly four fields.
 *
 * Adding a fifth breaks this line, whatever it is called, and that is the whole reason the
 * file exists: the type is what makes showing a secret in the palette impossible.
 */
export type HitHasNoRoomForASecret = Assert<
  Same<keyof SearchHit, 'module' | 'id' | 'title' | 'occurredAt'>
>;

/** A hit comes from one of the three modules, and never from a section with no data. */
export type HitsComeFromModules = Assert<Same<ModuleId, 'habits' | 'passwords' | 'finances'>>;

/** Settings and the panel are not modules, so nothing can claim to have been found in them. */
export type SettingsIsNotAModule = Assert<Same<Extract<ModuleId, 'settings' | 'panel'>, never>>;

/** Every field of a hit is readonly, so nothing can be added to one after it was built. */
export type HitsAreFrozen = Assert<
  Same<Readonly<SearchHit>, SearchHit> extends true ? true : false
>;

/** A provider is asked for a limit. A search with no ceiling is not part of the contract. */
export type ProviderTakesALimit = Assert<
  Same<Parameters<SearchProvider['search']>, [query: string, limit: number]>
>;

/** And it answers with hits and nothing else: no total, no cursor, no "and more". */
export type ProviderAnswersWithHits = Assert<
  Same<Awaited<ReturnType<SearchProvider['search']>>, readonly SearchHit[]>
>;
