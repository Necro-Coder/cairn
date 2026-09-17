/**
 * What a search result may contain, and what it may never contain.
 *
 * The command palette opens on two keys and can be opened by anybody sitting at an
 * unlocked window. It is also the one place in this application that will eventually reach
 * across habits, passwords and finances at once. Those two facts together are why the
 * shape of a result is written down here, in its own file, before there is anything to
 * search: the moment a provider exists, the temptation to return "just a little context"
 * alongside the title arrives with it.
 *
 * **A hit carries a module, an identity, a title and a date. There is no field for
 * content, for a value, for an amount or for a snippet, and there is no room to add one
 * without changing this file.** The type is what makes showing a secret impossible; the
 * discipline of whoever writes the provider is not. `contract.test-d.ts` fails to compile
 * if somebody adds one anyway.
 *
 * Nothing here crosses the boundary in this phase. Phase 03 puts a `search_titles` command
 * in the core, over titles decrypted into memory at unlock, and the provider becomes a
 * wrapper over it. Until then the three providers answer with nothing and the palette says
 * the part is in development.
 */

/** The three modules that hold data, which are the three that can be searched. */
export type ModuleId = 'habits' | 'passwords' | 'finances';

/**
 * One result.
 *
 * Four fields, and the absence of a fifth is the point of the type.
 */
export interface SearchHit {
  /** Which module it came from, which is how it is coloured and grouped. */
  readonly module: ModuleId;
  /**
   * What it is, so that choosing it can open it.
   *
   * Opaque to the interface: it is handed back to the module it came from and never
   * parsed, shown or built into a URL.
   */
  readonly id: string;
  /**
   * What is written on screen.
   *
   * A title and nothing else. For a password this is the name of the site, never the user
   * name and never the password. For a movement it is what it was called, never the
   * amount. For a habit it is the habit, never whether it was done.
   */
  readonly title: string;
  /**
   * When it happened, or null where the module has no date worth showing.
   *
   * A number of milliseconds since the epoch rather than a formatted string, so that the
   * interface decides how a date reads and a provider never has to.
   */
  readonly occurredAt: number | null;
}

/**
 * The most results a provider may ever return.
 *
 * A search without a ceiling is a search that in phase 05 hands the palette several
 * thousand titles to draw, over a list nobody scrolls past the first screen of. Fifty is
 * more than anybody reads and small enough that the cost of being wrong is nothing.
 */
export const MAX_HITS = 50;

/**
 * The longest query a provider is ever handed.
 *
 * The ceiling on results has a twin, and it is the one that matters more. In phase 03 this
 * string stops being matched against eight command titles in the WebView and starts being an
 * argument to a command in the core, which is to say an allocation in the process that holds
 * the keys, sized by whatever somebody pasted into a text field. Every input gets an explicit
 * length, and the place to write this one is the contract, before there is a provider to
 * forget it.
 *
 * Two hundred characters. The longest title in this application is four words, so anything
 * past this cannot match more than it already does; it can only cost more.
 */
export const MAX_QUERY = 200;

/** What a module has to offer for its data to be reachable from the palette. */
export interface SearchProvider {
  /** Which module this speaks for. */
  readonly module: ModuleId;
  /**
   * Answers a query with at most `limit` hits, best first.
   *
   * Both arguments arrive already bounded: `limit` is clamped to {@link MAX_HITS} and the
   * query to {@link MAX_QUERY} before they are passed, so a provider is never asked for more
   * than the ceilings above and never has to check. A provider that cannot answer
   * — because the vault closed underneath it, or because it is not written yet — returns
   * an empty list rather than throwing: one module being unavailable is not a reason for
   * the palette to show nothing.
   */
  search(query: string, limit: number): Promise<readonly SearchHit[]>;
}

/**
 * Brings a query within what the contract allows.
 *
 * Truncating rather than rejecting, because a query is not a command: somebody who pasted
 * too much wants the search to happen, and a search that refused to run would be reported as
 * the field being broken. Private for the same reason as {@link clampLimit} — the bound
 * belongs to the contract, not to whoever is calling.
 */
function clampQuery(query: string): string {
  return query.slice(0, MAX_QUERY);
}

/**
 * Brings a limit within what the contract allows.
 *
 * Private, and applied by `searchAll`, because the ceiling belongs to the contract rather
 * than to whoever is calling: a caller cannot ask for a hundred hits by forgetting to clamp,
 * and the caller phase 03 adds gets the same ceiling without having to know there was one.
 */
function clampLimit(limit: number): number {
  if (!Number.isFinite(limit)) {
    return MAX_HITS;
  }
  return Math.max(0, Math.min(MAX_HITS, Math.floor(limit)));
}

/**
 * Asks every provider at once and returns whatever answered.
 *
 * `allSettled` rather than `all`: a module that fails takes its own results with it and
 * nothing else. The palette showing two modules' worth of answers is better than the
 * palette showing an error because the third one was not ready.
 */
export async function searchAll(
  providers: readonly SearchProvider[],
  query: string,
  limit: number,
): Promise<readonly SearchHit[]> {
  const capped = clampLimit(limit);
  const asked = clampQuery(query);
  const answers = await Promise.allSettled(
    providers.map(async (provider) => provider.search(asked, capped)),
  );

  return answers
    .flatMap((answer) => (answer.status === 'fulfilled' ? [...answer.value] : []))
    .slice(0, capped);
}
