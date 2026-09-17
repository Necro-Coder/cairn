/**
 * The list of commands the palette offers.
 *
 * Separate from `commands.ts`, which holds the type and the matching, for one practical
 * reason: the five sections come from `sections.ts`, and `sections.ts` imports the icon
 * components. That makes it a module a browser can load and `node --test` cannot, so the
 * matching — the part worth testing — is kept where nothing imports a component.
 *
 * The sections are read from the same list the menu and the tab strip read, rather than
 * written out again here. A section added in a later phase appears in the palette without
 * anybody remembering that the palette existed.
 */

import { SECTIONS } from '../sections';
import type { Command } from './commands';

/** The actions that have no other home, and the sections, in the order they are offered. */
export const COMMANDS: readonly Command[] = [
  ...SECTIONS.map<Command>((section) => ({
    id: `open:${section.id}`,
    title: section.title,
    detail: `Abre ${section.title.toLocaleLowerCase('es')}`,
    group: 'Ir a',
    shortcut: section.shortcut,
    effect: { kind: 'open', section: section.id },
  })),
  {
    id: 'tab:close',
    title: 'Cerrar la pestaña actual',
    detail: 'Cierra la pestaña que estás viendo. El panel no se cierra',
    group: 'Pestañas',
    shortcut: 'Ctrl W',
    effect: { kind: 'closeTab' },
  },
  {
    id: 'tab:reopen',
    title: 'Reabrir la última pestaña cerrada',
    detail: 'Vuelve a abrirla, ya fijada',
    group: 'Pestañas',
    shortcut: 'Ctrl ⇧ T',
    effect: { kind: 'reopenTab' },
  },
  {
    id: 'vault:lock',
    title: 'Cerrar la caja fuerte',
    detail: 'Bloquea la aplicación ahora y descarta lo que tengas abierto',
    group: 'Caja fuerte',
    shortcut: 'Ctrl L',
    effect: { kind: 'lock' },
  },
];
