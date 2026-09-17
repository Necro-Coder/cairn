import { expect, test, type Page } from '@playwright/test';

import { expectNoViolations } from './axe';

/**
 * Every screen the preview build can reach, checked with `axe`.
 *
 * Driven rather than visited: there are no routes to type into an address bar, so each
 * screen is reached the way a person reaches it. That has a second benefit the URLs would
 * not have given — if a screen becomes unreachable, these fail.
 *
 * Nothing here asserts anything about the data. Preview mode invents all of it.
 */

/** Long enough for the policy the core enforces, and obviously not a real password. */
const PASSWORD = 'una frase larga de ejemplo';

/** Loads the application and waits until it has decided what to draw. */
async function open(page: Page): Promise<void> {
  await page.goto('/');
  // The first screen is drawn only after the interface has asked the boundary what the
  // state of the vault is. Running `axe` before that checks the waiting message.
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
}

/** Creates the vault, which is the only way to reach anything behind it. */
async function createVault(page: Page): Promise<void> {
  await page.getByLabel('Contraseña maestra', { exact: true }).fill(PASSWORD);
  await page.getByLabel('Repite la contraseña').fill(PASSWORD);
  await page.getByLabel(/Entiendo que si olvido/).check();
  await page.getByRole('button', { name: 'Crear la caja fuerte' }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Tu panel' })).toBeVisible();
}

/** Opens a section through the menu, which is how somebody with a mouse reaches one. */
async function openSection(page: Page, name: string): Promise<void> {
  await page.getByRole('button', { name: 'Abrir', exact: true }).click();
  await page.getByRole('menuitem', { name }).click();
}

test('creating the vault', async ({ page }) => {
  await open(page);
  await expectNoViolations(page, 'creating the vault');
});

test('the panel', async ({ page }) => {
  await open(page);
  await createVault(page);

  await expectNoViolations(page, 'the panel');
});

test('the open menu', async ({ page }) => {
  await open(page);
  await createVault(page);
  await page.getByRole('button', { name: 'Abrir', exact: true }).click();

  await expect(page.getByRole('menu', { name: 'Abrir' })).toBeVisible();
  await expectNoViolations(page, 'the open menu');
});

for (const [section, title] of [
  ['Hábitos', 'Hábitos'],
  ['Contraseñas', 'Contraseñas'],
  ['Finanzas', 'Finanzas'],
  ['Ajustes', 'Ajustes'],
] as const) {
  test(`the ${section.toLowerCase()} screen`, async ({ page }) => {
    await open(page);
    await createVault(page);
    await openSection(page, section);

    await expect(page.getByRole('heading', { level: 1, name: title })).toBeVisible();
    await expectNoViolations(page, `the ${section.toLowerCase()} screen`);
  });
}

test('a module explaining that it is in development', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openSection(page, 'Hábitos');
  // A button that is not wired up is still shown and still reacts. What it says when it is
  // pressed is part of the screen, so it is part of what gets checked.
  await page.getByRole('button', { name: 'Añadir hábito' }).click();

  await expect(page.getByText('En desarrollo', { exact: false })).toBeVisible();
  await expectNoViolations(page, 'a module in development');
});

test('the unlock screen', async ({ page }) => {
  await open(page);
  await createVault(page);
  await page.getByRole('button', { name: 'Cerrar la caja fuerte' }).click();

  await expect(page.getByRole('button', { name: 'Abrir', exact: true })).toBeVisible();
  await expectNoViolations(page, 'the unlock screen');
});

test('the diagnostics screen', async ({ page }) => {
  await open(page);
  await createVault(page);
  await page.keyboard.press('Control+Shift+KeyD');

  await expect(page.getByRole('heading', { level: 1, name: 'Diagnóstico' })).toBeVisible();
  await expectNoViolations(page, 'the diagnostics screen');
});
