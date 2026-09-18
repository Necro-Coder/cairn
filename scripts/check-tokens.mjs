/**
 * The gate that keeps the design system in the code.
 *
 * `docs/design/design-system.md` says that every colour, length and duration in the
 * interface comes from a token in `src/lib/styles/tokens.css`. A document cannot enforce
 * that. Erosion happens one reasonable exception at a time — a border here, a hard-coded
 * grey there — and by the time anybody notices, redoing it is a week of work. This is what
 * makes each exception argue for itself, in the diff, at the moment it is written.
 *
 * It has no dependencies, and that is deliberate rather than minimalism for its own sake.
 * The alternative considered was stylelint with `postcss-html` and the Svelte plugin,
 * which is three packages and their transitive graph to run three regular expressions
 * over a directory. What a short script solves does not become a dependency.
 *
 * What it looks for, under `src/`, in `.css` and `.svelte` files:
 *
 * 1. A colour written by hand: `#rrggbb`, `rgb(...)`, `hsl(...)` and the rest.
 * 2. A length in `px` or `rem`. Every one of them, not merely the ones off the scale: a
 *    component that writes `16px` is a component that has stopped reading the tokens,
 *    even when the number happens to be right today.
 * 3. A duration in `ms` or `s`.
 * 4. A `style` attribute in markup.
 *
 * The fourth is not about values at all, and it is here because this is the file that reads
 * every component. The content security policy in `tauri.conf.json` sets `style-src 'self'`
 * with no `unsafe-inline`, and that blocks inline style attributes as well as inline `<style>`
 * elements. A browser with no policy applied honours them, so the preview cannot show the
 * fault: the declaration is simply dropped in the real window and the colour, the size or the
 * edge it carried disappears. Module colours reach a component through a class declared in
 * `base.css`; see the note there.
 *
 * What it deliberately does not look for: `em`, `ch`, `%`, `fr` and the viewport units.
 * Those are relative to something the component already has, so they are a way of saying
 * "the same as its text" rather than a value typed in from nowhere.
 *
 * Two things are exempt.
 *
 * `src/lib/styles/tokens.css` is the file the values live in, which is the whole point of
 * there being one file.
 *
 * A media query prelude may contain a length, because a media query cannot read a custom
 * property. There are exactly two breakpoints in this application and both are named in
 * the design system.
 *
 * Anything else needs a comment on the line above saying `tokens-exempt: <reason>`. The
 * list of those starts empty and every addition is visible in review.
 */

import { readdir, readFile } from 'node:fs/promises';
import { argv, exit, stderr, stdout } from 'node:process';
import { join, relative, sep } from 'node:path';

/** Where the interface lives. Everything outside it is somebody else's toolchain. */
const ROOT = 'src';

/** The one file that is allowed to contain values, because it is the values. */
const TOKENS_FILE = join('src', 'lib', 'styles', 'tokens.css');

/** What a component is written in. Nothing else under `src/` can carry a style. */
const EXTENSIONS = ['.css', '.svelte'];

/**
 * The comment that lets one line through, and which has to say why.
 *
 * The lookahead is what makes the reason compulsory rather than decorative: without it,
 * `tokens-exempt:` followed immediately by the end of the comment satisfies "some
 * non-space character" using the comment terminator itself.
 */
const EXEMPTION = /tokens-exempt:\s*(?!\*\/|-->)\S/;

/**
 * The four things a component may not contain.
 *
 * Each one is reported with the name of the token family the author was supposed to
 * reach for, because a gate that only says "no" teaches nobody where to look.
 */
const RULES = [
  {
    name: 'colour',
    // `#abc`, `#aabbcc`, `#aabbccdd`, and every functional notation CSS has for a colour.
    pattern: /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\s*\(/g,
    advice: 'use a --colour-*, --module-* or --mark-* token',
  },
  {
    name: 'length',
    // A number with an absolute unit. `1e3px` is not valid CSS, so no exponent to handle.
    pattern: /(?<![\w-])\d*\.?\d+(?:px|rem)\b/g,
    advice: 'use a --space-*, --text-*, --radius-* or other length token',
  },
  {
    name: 'duration',
    // Anchored on the left so that `0.925em` and a word ending in `s` cannot match.
    pattern: /(?<![\w.-])\d*\.?\d+m?s\b/g,
    advice: 'use --duration-fast or --duration-normal',
  },
  {
    name: 'inline-style',
    // A `style` attribute in markup, whether it is given a string or a Svelte expression.
    pattern: /(?<![\w-])style=["{]/g,
    advice:
      'the policy drops inline styles in the real window; put the values on a class in base.css',
  },
];

/**
 * Blanks out every media query prelude, leaving the text the same length.
 *
 * Replacing rather than removing keeps every later offset honest, so a violation is
 * reported on the line it is actually on. A prelude is allowed to carry a length because
 * a media query cannot read a custom property; there is no way to write the two
 * breakpoints of this application other than as numbers.
 */
function blankMediaPreludes(source) {
  return source.replace(/@media[^{]*/g, (prelude) => ' '.repeat(prelude.length));
}

/**
 * Blanks out every comment, for the same reason and in the same way.
 *
 * A comment explaining why a value is what it is will name the value. Reporting that as a
 * violation would teach people to stop writing the explanation, which is the opposite of
 * what this file is for.
 */
function blankComments(source) {
  return source.replace(/\/\*[\s\S]*?\*\/|<!--[\s\S]*?-->/g, (comment) =>
    comment.replace(/[^\n]/g, ' '),
  );
}

/**
 * Finds every hand-written value in one file's text.
 *
 * Pure, and exported, so the gate can be tested against strings rather than against a
 * directory somebody has to keep in a particular state. It is the only function here with
 * any judgement in it, and it is the one the tests point at.
 *
 * @param {string} source the contents of the file
 * @param {string} [path] what to call it in a report
 * @returns {{path: string, line: number, column: number, rule: string, text: string, advice: string}[]}
 */
export function findViolations(source, path = '<source>') {
  const searchable = blankMediaPreludes(blankComments(source));
  const lines = searchable.split('\n');
  const original = source.split('\n');
  const violations = [];

  for (const [index, line] of lines.entries()) {
    // The exemption is read from the original text, because the line above is a comment
    // and comments have been blanked out of the searchable copy by this point.
    const previous = original[index - 1] ?? '';
    if (EXEMPTION.test(previous)) {
      continue;
    }

    for (const rule of RULES) {
      // Reset rather than construct: a global regular expression carries its own cursor,
      // and reusing one across lines without this skips matches at random.
      rule.pattern.lastIndex = 0;
      let match;
      while ((match = rule.pattern.exec(line)) !== null) {
        violations.push({
          path,
          line: index + 1,
          column: match.index + 1,
          rule: rule.name,
          text: match[0],
          advice: rule.advice,
        });
      }
    }
  }

  return violations;
}

/** Every file under a directory, depth first, in a stable order. */
async function filesUnder(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const found = [];

  for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      found.push(...(await filesUnder(path)));
    } else if (EXTENSIONS.some((extension) => entry.name.endsWith(extension))) {
      found.push(path);
    }
  }

  return found;
}

/**
 * Runs the gate over the interface and reports.
 *
 * Exits non-zero on the first file with a violation in it, but only after listing every
 * violation in every file: fixing them one build at a time is how a gate becomes a thing
 * people resent.
 */
async function main() {
  const paths = (await filesUnder(ROOT)).filter(
    (path) => relative(TOKENS_FILE, path) !== '' && path !== TOKENS_FILE,
  );

  const violations = [];
  for (const path of paths) {
    violations.push(...findViolations(await readFile(path, 'utf8'), path.split(sep).join('/')));
  }

  if (violations.length === 0) {
    stdout.write(`check-tokens: ${String(paths.length)} files, no hand-written values.\n`);
    return;
  }

  for (const violation of violations) {
    stderr.write(
      `${violation.path}:${String(violation.line)}:${String(violation.column)} ` +
        `${violation.rule} literal \`${violation.text}\` — ${violation.advice}\n`,
    );
  }
  stderr.write(
    `\ncheck-tokens: ${String(violations.length)} hand-written value(s). ` +
      'Every colour, length and duration comes from src/lib/styles/tokens.css. ' +
      'If one genuinely cannot, put `tokens-exempt: <reason>` in a comment on the line above.\n',
  );
  exit(1);
}

// Only when run as a command. Imported by its test, which wants the function and not the
// exit code.
if (argv[1]?.endsWith('check-tokens.mjs') === true) {
  await main();
}
