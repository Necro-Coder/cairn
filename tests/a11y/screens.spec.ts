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
  // Habits is not in this list any more. The module works, so nothing on its screen is drawn
  // before it does anything, and the badge that says so came off with the mock-up.
  //
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

test('the habits list, with what today asks for on it', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openSection(page, 'Hábitos');

  // Every row is a control with a name that says what pressing it does, which is the half
  // of this screen a mouse never exercises.
  await expect(page.getByRole('button', { name: 'Marcar hoy en Meditar' })).toBeVisible();
  await expectNoViolations(page, 'the habits list');
});

test('one habit opened, with its year and its numbers', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openSection(page, 'Hábitos');
  await page.getByRole('button', { name: 'Abrir Meditar' }).click();

  // The year is a figure with a caption rather than a grid announced cell by cell, and the
  // arrows are the half of it that has to be reachable from the keyboard.
  await expect(page.getByRole('heading', { level: 1, name: 'Meditar' })).toBeVisible();
  await expect(page.getByText(/Un cuadro por cada día de/)).toBeVisible();
  await expectNoViolations(page, 'one habit opened');
});

test('stepping back a year, which asks for one more calendar and nothing else', async ({
  page,
}) => {
  await open(page);
  await createVault(page);
  await openSection(page, 'Hábitos');
  await page.getByRole('button', { name: 'Abrir Meditar' }).click();
  await expect(page.getByText(/Un cuadro por cada día de/)).toBeVisible();

  const year = new Date().getUTCFullYear();
  const before = year - 1;
  await page.getByRole('button', { name: String(before) }).click();

  await expect(
    page.getByRole('heading', { level: 2, name: `El año ${String(before)}` }),
  ).toBeVisible();
  await expect(page.getByText(new RegExp(`cada día de ${String(before)}`))).toBeVisible();
  await expectNoViolations(page, 'one habit, a year back');
});

test('the habits list with nothing in it', async ({ page }) => {
  // The first day of a list is a real screen, and until the screen that deletes a habit
  // exists there is no way to reach it from inside the interface. The preview opens on it
  // when the address asks, which is the one switch that file has.
  await page.goto('/?sin-habitos');
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await createVault(page);
  await openSection(page, 'Hábitos');

  await expect(page.getByText(/Hoy no hay nada que marcar/)).toBeVisible();
  await expectNoViolations(page, 'the habits list, empty');
});

test('the form for a habit that does not exist yet', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openSection(page, 'Hábitos');
  await page.getByRole('button', { name: 'Añadir hábito' }).click();

  await expect(page.getByRole('heading', { level: 1, name: 'Nuevo hábito' })).toBeVisible();
  // Every control has a label of its own, and the fields that only belong to a habit
  // counting a quantity are absent rather than disabled.
  await expect(page.getByLabel('Nombre')).toBeVisible();
  await expect(page.getByLabel('Objetivo por período')).toHaveCount(0);
  await expectNoViolations(page, 'the new habit form');
});

test('the form with the quantity fields on it', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openSection(page, 'Hábitos');
  await page.getByRole('button', { name: 'Añadir hábito' }).click();

  // Naming a unit is what brings the other two out. They appear rather than un-dim.
  await page.getByLabel('Unidad').fill('ml');

  await expect(page.getByLabel('Objetivo por período')).toBeVisible();
  await expectNoViolations(page, 'the new habit form, counting a quantity');
});

test('the form on a habit that already exists', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openSection(page, 'Hábitos');
  await page.getByRole('button', { name: 'Abrir Meditar' }).click();
  await page.getByRole('button', { name: 'Editar' }).click();

  await expect(page.getByRole('heading', { level: 1, name: 'Editar hábito' })).toBeVisible();
  await expect(page.getByLabel('Nombre')).toHaveValue('Meditar');
  await expectNoViolations(page, 'the edit habit form');
});

test('the warning that says what changing a habit would mean', async ({ page }) => {
  await open(page);
  await createVault(page);
  await openSection(page, 'Hábitos');
  await page.getByRole('button', { name: 'Abrir Meditar' }).click();
  await page.getByRole('button', { name: 'Editar' }).click();
  await expect(page.getByLabel('Nombre')).toHaveValue('Meditar');

  // Turning the habit round is exactly the change that alters what the run means.
  await page.getByLabel('Quiero evitarlo, como mucho').check();
  await page.getByRole('button', { name: 'Guardar los cambios' }).click();

  const warning = page.getByRole('dialog');
  await expect(warning).toBeVisible();
  await expect(warning.getByText(/No se ha guardado nada todavía/)).toBeVisible();
  await expectNoViolations(page, 'the warning before saving');

  // Escape is the same as cancelling, which is the promise a dialog makes.
  await page.keyboard.press('Escape');
  await expect(warning).toHaveCount(0);
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

/**
 * The screen a second copy of the application draws.
 *
 * Reached by a query parameter rather than by driving the interface, because there is no
 * sequence of actions inside one browser tab that produces a second process. It is the only
 * screen in this file reached that way, and the reason is written here so that nobody takes it
 * as a pattern for the rest.
 */
for (const [state, heading] of [
  ['alreadyRunning', 'Ya hay una copia abierta'],
  ['unavailable', 'No se ha podido reservar la carpeta de datos'],
  ['noDirectory', 'No hay dónde guardar la caja'],
] as const) {
  test(`the ${state} refusal`, async ({ page }) => {
    await page.goto(`/?instance=${state}`);

    await expect(page.getByRole('heading', { level: 1, name: heading })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Cerrar esta ventana' })).toBeVisible();
    await expectNoViolations(page, `the ${state} refusal`);
  });
}
