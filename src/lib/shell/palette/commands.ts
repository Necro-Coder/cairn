/**
 * What the command palette can do, and how a query picks one.
 *
 * The matching is here, over plain values, rather than inside the component: it is the part
 * that has to be right — an accent that stops a match, a query that matches everything, an
 * order that puts the thing somebody runs twenty times a day third — and none of that needs
 * a window to check.
 *
 * Accents are folded before anything is compared. The interface is in Spanish, and a palette
 * where `habitos` does not find `Hábitos` is a palette somebody stops using on the second
 * day, because typing the accent is slower than reaching for the mouse.
 */

import type { SectionId } from '../sections';

/** What running a command does, from the point of view of everything outside the palette. */
type CommandEffect =
  | { readonly kind: 'open'; readonly section: SectionId }
  | { readonly kind: 'closeTab' }
  | { readonly kind: 'reopenTab' }
  | { readonly kind: 'lock' };

/** One thing the palette can run. */
export interface Command {
  /** Stable, and what the history is kept by. */
  readonly id: string;
  /** What is written on the row. */
  readonly title: string;
  /** One short line under it, saying what it does. */
  readonly detail: string;
  /** The heading it is listed under. */
  readonly group: string;
  /** The keys that do the same thing without the palette, where there are any. */
  readonly shortcut: string | null;
  /** What running it does. */
  readonly effect: CommandEffect;
}

/**
 * Text as it is compared: lower case, without accents, without leading or trailing space.
 *
 * `NFD` splits an accented letter into the letter and its mark, and the mark is then thrown
 * away, which leaves the bare letter. It is the whole of the accent handling and it costs
 * one line.
 */
export function fold(text: string): string {
  return text
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .toLocaleLowerCase('es')
    .trim();
}

/**
 * How well a command answers a query, or null when it does not.
 *
 * Three degrees, and they are what decide the order: the title starts with what was typed,
 * a word of the title starts with it, or it appears anywhere in the title or the detail.
 * Anything cleverer — fuzzy subsequences, edit distance — would put rows that are not what
 * somebody typed above rows that are, over a list of nine entries.
 */
export function score(command: Command, query: string): number | null {
  const wanted = fold(query);
  if (wanted === '') {
    return 0;
  }

  const title = fold(command.title);
  if (title.startsWith(wanted)) {
    return 3;
  }
  if (title.split(' ').some((word) => word.startsWith(wanted))) {
    return 2;
  }
  if (title.includes(wanted) || fold(command.detail).includes(wanted)) {
    return 1;
  }
  return null;
}

/**
 * The commands that answer a query, best first.
 *
 * With nothing typed the order is what was run recently and then the catalogue, which is
 * what makes the palette worth opening: the thing done every morning is at the top before a
 * key is pressed. With something typed the history is ignored, because somebody who has
 * typed three letters is looking for what they typed.
 */
export function rank(
  commands: readonly Command[],
  query: string,
  history: readonly string[],
): readonly Command[] {
  if (fold(query) === '') {
    const recent = history
      .map((id) => commands.find((command) => command.id === id))
      .filter((command): command is Command => command !== undefined);

    return [...recent, ...commands.filter((command) => !history.includes(command.id))];
  }

  return commands
    .map((command) => ({ command, score: score(command, query) }))
    .filter((hit): hit is { command: Command; score: number } => hit.score !== null)
    .sort((a, b) => b.score - a.score)
    .map((hit) => hit.command);
}
