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

for (const [section, action, said] of [
  ['Hábitos', 'Añadir hábito', /Crear y marcar hábitos llega/],
  // Named one by one rather than matched loosely: the passwords screen says the same thing
  // twice, once about the search field and once about the list, and a pattern that caught
  // both would be a test that passed while the button did nothing.
  ['Contraseñas', 'Añadir contraseña', /Guardar y leer contraseñas llega/],
  ['Finanzas', 'Añadir movimiento', /Registrar movimientos llega/],
] as const) {
  test(`${section.toLowerCase()} explaining that the part is in development`, async ({ page }) => {
    await open(page);
    await createVault(page);
    await openSection(page, section);
    // A button that is not wired up is still shown and still reacts. What it says when it is
    // pressed is part of the screen, so it is part of what gets checked.
    await page.getByRole('button', { name: action }).click();

    await expect(page.getByText(said)).toBeVisible();
    await expectNoViolations(page, `${section.toLowerCase()} in development`);
  });
}

test('the habits year, drawn with nothing in it', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openSection(page, 'Hábitos');

  // The grid itself is hidden from assistive technology, so what has to be reachable is
  // the sentence that says what it is. A grid nobody can have described to them, with no
  // caption, would be three hundred squares of nothing.
  await expect(page.getByRole('heading', { level: 2, name: 'Tu año' })).toBeVisible();
  await expect(page.getByText(/Un cuadro por día/)).toBeVisible();
  await expectNoViolations(page, 'the habits year');
});

test('the passwords search, switched off with its reason', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openSection(page, 'Contraseñas');

  // A disabled control with no explanation is a bug report waiting to be filed, so the
  // reason is on screen and not only in the title attribute.
  await expect(page.getByLabel('Buscar')).toBeDisabled();
  await expect(page.getByText(/Buscar entre las contraseñas llega/)).toBeVisible();
  await expectNoViolations(page, 'the passwords search');
});

test('the finances summary, at zero', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openSection(page, 'Finanzas');

  await expect(page.getByRole('heading', { level: 2, name: 'Este mes' })).toBeVisible();
  await expect(page.getByText(/no hay ningún movimiento/)).toBeVisible();
  await expectNoViolations(page, 'the finances summary');
});

test('the unlock screen', async ({ page }) => {
  await open(page);
  await createVault(page);
  await page.getByRole('button', { name: 'Cerrar la caja fuerte' }).click();

  await expect(page.getByRole('button', { name: 'Abrir', exact: true })).toBeVisible();
  await expectNoViolations(page, 'the unlock screen');
});

test('the diagnostics, inside settings', async ({ page }) => {
  await open(page);
  await createVault(page);
  await page.keyboard.press('Control+Shift+KeyD');

  // The shortcut opens the settings tab at the right part rather than a panel of its own.
  await expect(page.getByRole('heading', { level: 1, name: 'Ajustes' })).toBeVisible();
  await expect(page.getByRole('heading', { level: 2, name: 'Este equipo' })).toBeVisible();
  await expectNoViolations(page, 'the diagnostics inside settings');
});

test('the diagnostics on their own, with the vault closed', async ({ page }) => {
  await open(page);
  await createVault(page);
  await page.getByRole('button', { name: 'Cerrar la caja fuerte' }).click();
  await page.keyboard.press('Control+Shift+KeyD');

  await expect(page.getByRole('heading', { level: 1, name: 'Qué es esta copia' })).toBeVisible();
  await expectNoViolations(page, 'the diagnostics on their own');
});

for (const part of ['Seguridad', 'Apariencia', 'Datos', 'Atajos'] as const) {
  test(`the ${part.toLowerCase()} part of settings`, async ({ page }) => {
    await open(page);
    await createVault(page);
    await openSection(page, 'Ajustes');
    await page.getByRole('button', { name: part, exact: true }).click();

    await expectNoViolations(page, `settings: ${part}`);
  });
}

/** Opens the palette the way it is meant to be opened. */
async function openPalette(page: Page): Promise<void> {
  await page.keyboard.press('Control+KeyK');
  await expect(page.getByRole('dialog', { name: 'Paleta de comandos' })).toBeVisible();
}

test('the tab strip with several tabs open', async ({ page }) => {
  await open(page);
  await createVault(page);

  await openSection(page, 'Hábitos');
  // A double click is what makes a tab permanent, so the strip under test holds one of
  // each: the panel, a pinned tab and the temporary one.
  await page.getByRole('button', { name: 'Hábitos', exact: true }).dblclick();
  await openSection(page, 'Finanzas');

  await expect(page.getByRole('navigation', { name: 'Pestañas abiertas' })).toBeVisible();
  await expectNoViolations(page, 'the tab strip');
});

test('the card picker', async ({ page }) => {
  await open(page);
  await createVault(page);
  await page.getByRole('button', { name: 'Añadir tarjeta' }).click();

  await expect(page.getByRole('menu', { name: 'Tarjetas disponibles' })).toBeVisible();
  await expectNoViolations(page, 'the card picker');
});

test('the panel with cards on it', async ({ page }) => {
  await open(page);
  await createVault(page);
  await page.getByRole('button', { name: 'Añadir tarjeta' }).click();
  await page.getByRole('menuitemcheckbox', { name: /Resumen del mes/ }).click();
  await page.keyboard.press('Escape');

  await expect(page.getByRole('heading', { level: 2, name: 'Resumen del mes' })).toBeVisible();
  await expectNoViolations(page, 'the panel with cards');
});

test('the command palette', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openPalette(page);

  await expectNoViolations(page, 'the command palette');
});

test('the command palette with something typed', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openPalette(page);
  // Without the accent, on purpose: the fold is what makes the palette usable in Spanish.
  await page.getByLabel('Buscar o ejecutar').fill('habitos');

  await expect(page.getByRole('option', { name: /Hábitos/ })).toBeVisible();
  await expectNoViolations(page, 'the command palette with a query');
});
