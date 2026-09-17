import { expect, type Page } from '@playwright/test';
import type * as axeCore from 'axe-core';

/**
 * Running `axe` against a page, and failing with something a person can act on.
 *
 * `axe-core` is loaded as a script into the page rather than through its Playwright
 * wrapper, which is one more package for a job that is two lines. The file comes from the
 * installed package, so the rules and the version are the ones the lockfile pins.
 */

declare global {
  interface Window {
    /** Injected by {@link expectNoViolations}. It is not there before that. */
    readonly axe: typeof axeCore;
  }
}

/**
 * The rule sets this project is held to.
 *
 * WCAG 2.2 at level AA, and the best practice set on top of it. The second one is not
 * required by any standard and is included on purpose: it is where "every region has a
 * landmark" and "headings are in order" live, and both are things that are cheap now and
 * expensive to retrofit across fifteen screens.
 */
const RULE_SETS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'];

/**
 * Asserts that a page, as it currently stands, has no accessibility violation.
 *
 * Called after the screen has been driven into the state being checked, never on a page
 * that is still loading: `axe` reports what is on screen at the moment it runs, so a
 * half-drawn screen produces a clean report about nothing.
 *
 * @param page the page to check
 * @param what the screen being checked, for the failure message
 */
export async function expectNoViolations(page: Page, what: string): Promise<void> {
  // Resolved from the working directory. The gate always runs from the repository root,
  // which is also where `npm` puts the package.
  await page.addScriptTag({ path: 'node_modules/axe-core/axe.min.js' });

  const violations = await page.evaluate(async (ruleSets) => {
    const results = await window.axe.run(document, { runOnly: ruleSets });
    return results.violations.map((violation) => ({
      id: violation.id,
      impact: violation.impact,
      help: violation.help,
      // The first three are enough to find it. A full list of nodes for a rule that
      // matches a whole list turns a failure into a wall nobody reads.
      where: violation.nodes.slice(0, 3).map((node) => node.target.join(' ')),
    }));
  }, RULE_SETS);

  expect(
    violations,
    `${what}: axe reported ${String(violations.length)} violation(s)\n` +
      violations
        .map(
          (violation) =>
            `  ${violation.id} (${violation.impact ?? 'unknown'}): ${violation.help}\n` +
            violation.where.map((target) => `    at ${target}`).join('\n'),
        )
        .join('\n'),
  ).toEqual([]);
}
