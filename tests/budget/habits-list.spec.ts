import { expect, test, type Page } from '@playwright/test';

/**
 * How long the habits list takes to appear when there is a lot of it.
 *
 * Not a benchmark. A benchmark asks how fast a screen is on this machine, which is a number
 * that changes with the machine and cannot be a blocking check. This asks a different
 * question, one with a yes and a no: does the screen do an amount of work proportional to the
 * number of rows, or proportional to the square of it? The two answers are not near each
 * other, so no machine is fast or slow enough to confuse them.
 *
 * The screen that prompted this walked its whole list once per row to find out which row was
 * the last one. Measured in this browser, on ten thousand habits: 137 seconds before, 5,5
 * after. The budget below sits between the two with a factor of eight of room underneath it
 * and a factor of three above, which is what makes it a gate rather than a flaky
 * measurement — a machine would have to be eight times slower than this one to fail it
 * honestly, and a quadratic screen three times faster than this one to pass it dishonestly.
 *
 * Ten thousand is not a number of habits anybody has. It is the number the core's own seeding
 * command writes into a vault from the diagnostics screen, which is how the fault was found,
 * and the number the preview stand-in will invent when the address asks it to.
 */

/** Long enough for the policy the core enforces, and obviously not a real password. */
const PASSWORD = 'una frase larga de ejemplo';

/** How many habits the stand-in is asked to invent. */
const MANY = 10_000;

/**
 * The longest the list may take from the press that opens it to its first row being there.
 *
 * Measured, not guessed. See the note at the top of this file for the two numbers it sits
 * between.
 */
const BUDGET_MS = 45_000;

/** Creates the vault, which is the only way to reach anything behind it. */
async function createVault(page: Page): Promise<void> {
  await page.getByLabel('Contraseña maestra', { exact: true }).fill(PASSWORD);
  await page.getByLabel('Repite la contraseña').fill(PASSWORD);
  await page.getByLabel(/Entiendo que si olvido/).check();
  await page.getByRole('button', { name: 'Crear la caja fuerte' }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Tu panel' })).toBeVisible();
}

test('a list of ten thousand habits is drawn once, not once per row', async ({ page }) => {
  // The default is thirty seconds, which is under the budget this test is about.
  test.setTimeout(BUDGET_MS * 3);

  await page.goto(`/?muchos=${String(MANY)}`);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await createVault(page);

  await page.getByRole('button', { name: 'Abrir', exact: true }).click();

  // The clock starts at the press and stops at the first row, because that is the whole of
  // what somebody waits through. Everything before the press is the panel, which is measured
  // by nothing here and is the same on every one of these tests.
  const started = Date.now();
  await page.getByRole('menuitem', { name: 'Hábitos' }).click();
  await expect(page.getByRole('button', { name: 'Marcar hoy en Hábito de prueba 0' })).toBeVisible({
    timeout: BUDGET_MS,
  });
  const elapsed = Date.now() - started;

  // Asserted as well as waited for, so a failure says the number rather than only that
  // something timed out.
  expect(elapsed, `the list of ${String(MANY)} habits took ${String(elapsed)} ms`).toBeLessThan(
    BUDGET_MS,
  );

  // Every one of them, not a page of them. The screen does not paginate, and a budget met by
  // drawing the first ten rows would be a budget met by changing the subject. Counted by the
  // control each row carries rather than by the row, because a list item is a shape several
  // parts of the shell use and the mark button belongs to this screen alone.
  await expect(page.getByRole('button', { name: /^Marcar hoy en Hábito de prueba / })).toHaveCount(
    MANY,
  );
});
