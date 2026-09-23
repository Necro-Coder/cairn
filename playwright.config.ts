import { defineConfig, devices } from '@playwright/test';

/**
 * What only a real browser can check.
 *
 * Two suites, and both are here for the same reason. `tests/a11y` is the accessibility
 * gate: WCAG 2.2 AA is the floor this project committed to, and a floor nothing measures
 * is a floor that sinks one screen at a time, so `axe` runs over every screen in both
 * themes as a blocking check. `tests/budget` is the other half of the same argument — a
 * screen that takes two minutes to appear has failed whatever `axe` says about it, and how
 * long a screen takes to appear is not a question a fake DOM can be asked.
 *
 * A real browser rather than `jsdom`, and that is the whole reason Playwright is here at
 * all. Half of the AA rules are about computed colour and computed layout: contrast,
 * overlap, target size, focus visibility. `jsdom` has neither, so it evaluates none of
 * them and reports a clean run. A gate that passes without having looked is worse than no
 * gate, because it is believed.
 *
 * Playwright and `axe-core` are **development** dependencies. `package.json` still
 * declares no runtime dependencies, and that is its own gate.
 */
export default defineConfig({
  testDir: './tests',
  // Nothing here shares state with anything else, and a browser is cheap to start.
  fullyParallel: true,
  // A test that only passes on the second attempt has told us something, and retrying
  // would hide it. There is no network and no real backend here, so there is nothing
  // legitimately flaky to absorb.
  retries: 0,
  // `.only` left in a file would quietly reduce the gate to one screen.
  forbidOnly: true,
  reporter: [['list']],

  use: {
    baseURL: 'http://127.0.0.1:1420',
    // Only on a failure: a trace for a run that passed is a hundred megabytes nobody opens.
    trace: 'retain-on-failure',
  },

  /*
   * The same screens twice, once in each theme.
   *
   * Both are drawn from the same tokens, but they are two different sets of values and
   * only one of them can be the one somebody checked by eye. Contrast in particular is a
   * property of the pair, so a palette that passes on paper says nothing about ink.
   */
  projects: [
    {
      name: 'paper',
      use: { ...devices['Desktop Chrome'], colorScheme: 'light' },
    },
    {
      name: 'ink',
      // The accessibility suite alone. What a screen costs to draw is the same in both
      // palettes, so running the budget suite twice would double the slowest check in the
      // pipeline to learn nothing, and the two copies would compete for the same cores
      // while being timed.
      testDir: './tests/a11y',
      use: { ...devices['Desktop Chrome'], colorScheme: 'dark' },
    },
  ],

  /*
   * Preview mode, which is the only way to reach these screens without a Rust core: the
   * `$ipc` alias points at the module that invents its answers. Nothing it returns is
   * real, and nothing here asserts that it is — what is being checked is the markup and
   * the computed style of the screens, which are the same either way.
   */
  webServer: {
    command: 'npm run preview:mock',
    url: 'http://127.0.0.1:1420',
    // Locally there is often one already running from looking at the interface.
    reuseExistingServer: true,
    stdout: 'ignore',
    stderr: 'pipe',
  },
});
