/**
 * The five parts of the settings screen, as they are read.
 *
 * The identifiers live in `src/lib/shell/workspace.ts`, because which one is open is part of
 * the workspace and the workspace cannot depend on a screen. What is here is what each one
 * is called and what it is for — the part that belongs to the interface.
 */

import type { SettingsSectionId } from '../../lib/shell/workspace';

/** One part of the screen. */
export interface SettingsSection {
  readonly id: SettingsSectionId;
  /** What the chooser says. */
  readonly title: string;
  /** One line under the title of the screen, saying what this part is about. */
  readonly lede: string;
}

/**
 * The five, in the order they are offered.
 *
 * Security first because it is the one somebody comes here for, and shortcuts last because
 * it is the one nobody comes here for and everybody reads once.
 */
export const SETTINGS_SECTIONS: readonly SettingsSection[] = [
  {
    id: 'security',
    title: 'Seguridad',
    lede: 'La contraseña maestra, los parámetros de derivación y cuándo se cierra sola.',
  },
  {
    id: 'appearance',
    title: 'Apariencia',
    lede: 'Papel o tinta, o lo que diga el sistema.',
  },
  {
    id: 'data',
    title: 'Datos',
    lede: 'Copias, importar y exportar. Todavía no hay nada que copiar.',
  },
  {
    id: 'diagnostics',
    title: 'Diagnóstico',
    lede: 'Qué versión es esta y cómo se está portando, sin nada que identifique al equipo.',
  },
  {
    id: 'shortcuts',
    title: 'Atajos',
    lede: 'Todo lo que se puede hacer sin tocar el ratón.',
  },
];

/**
 * What a part of the screen is.
 *
 * Total over `SettingsSectionId`, like `sectionOf`: the type guarantees the answer exists,
 * so no caller has a "not found" branch to get wrong.
 */
export function settingsSectionOf(id: SettingsSectionId): SettingsSection {
  const found = SETTINGS_SECTIONS.find((section) => section.id === id);
  if (found === undefined) {
    // Unreachable while the identifiers and this list agree. If it fires, one lost an entry.
    throw new Error(`el apartado «${id}» no está en la lista`);
  }
  return found;
}
