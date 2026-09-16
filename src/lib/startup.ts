/**
 * Startup measurements.
 *
 * Two of the project's performance budgets can be measured from the moment the shell
 * exists, before there is anything in it to slow it down. Taking the baseline now means
 * that when the application later grows, there is a real number to compare against
 * instead of an argument about whether it used to feel faster.
 *
 * These are measurements of this build on this machine. They are shown on the diagnostics
 * screen and nowhere else, and they are never sent anywhere, because nothing in this
 * application is ever sent anywhere.
 */

import { fetchAppInfo, fetchDiagnostics, type AppInfo } from './ipc';

/** A timing, in milliseconds, together with the budget it is being held to. */
export interface Measurement {
  readonly milliseconds: number;
  readonly budgetMilliseconds: number;
}

/** Whether a measurement came in under its budget. */
export function isWithinBudget(measurement: Measurement): boolean {
  return measurement.milliseconds <= measurement.budgetMilliseconds;
}

/** What the first calls into the core produced. */
export interface StartupResult {
  readonly appInfo: AppInfo;
  /**
   * Process start to the first answer the interface received. Budget: 400 ms.
   *
   * Taken from the uptime the core reports rather than from a timer in here, because the
   * browser clock starts when the document does, which is already most of the way through
   * startup. The core has been running since before the window existed, so its uptime is
   * the only number that covers the whole wait: process, window, WebView, bundle and one
   * round trip.
   */
  readonly coldStart: Measurement;
  /**
   * A warm round trip across the command boundary. Budget: 5 ms.
   *
   * The median of several calls, not the first one. The first call pays for setting up
   * the channel, so reporting it would mean reporting the cost of starting up twice and
   * calling the second one a latency problem.
   */
  readonly commandLatency: Measurement;
}

const COLD_START_BUDGET_MS = 400;
const COMMAND_LATENCY_BUDGET_MS = 5;

/** How many warm calls to time before taking the median. */
const LATENCY_SAMPLES = 9;

/** The middle value of a list, which ignores a single slow outlier the way a mean cannot. */
function median(values: readonly number[]): number {
  if (values.length === 0) {
    return Number.NaN;
  }
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  const value =
    sorted.length % 2 === 0
      ? ((sorted[middle - 1] ?? 0) + (sorted[middle] ?? 0)) / 2
      : sorted[middle];
  return value ?? Number.NaN;
}

/** Rounds to two decimals, because a sub-millisecond round trip is real and worth seeing. */
function round(value: number): number {
  return Math.round(value * 100) / 100;
}

/**
 * Measures startup and command latency.
 *
 * Called once when the application mounts, not from a button, so that the cold start
 * figure is the time the person actually waited rather than however long they took to
 * reach for the mouse.
 */
export async function measureStartup(): Promise<StartupResult> {
  // The first call doubles as the cold start measurement and as the channel warm-up.
  const snapshot = await fetchDiagnostics();

  const samples: number[] = [];
  for (let attempt = 0; attempt < LATENCY_SAMPLES; attempt += 1) {
    const startedAt = performance.now();
    // Sequential on purpose: running them at once would measure how well the runtime
    // parallelises, not how long one command takes.
    await fetchAppInfo();
    samples.push(performance.now() - startedAt);
  }

  return {
    appInfo: snapshot.app,
    coldStart: {
      milliseconds: snapshot.uptimeMs,
      budgetMilliseconds: COLD_START_BUDGET_MS,
    },
    commandLatency: {
      milliseconds: round(median(samples)),
      budgetMilliseconds: COMMAND_LATENCY_BUDGET_MS,
    },
  };
}
