/**
 * The places this application can open, and what each one looks like.
 *
 * One list, read by the menu, the tab strip, the command palette and the keyboard
 * shortcuts. Four copies of it would be four places to forget when a section is added, and
 * three of them would be wrong for a while without anything saying so.
 *
 * What is here is presentation: a title, a colour, an icon, a shortcut. What a section
 * actually draws is a screen, and a screen knows nothing about any of this.
 */

import type { Component } from 'svelte';

import IconFinances from '../icons/IconFinances.svelte';
import IconHabits from '../icons/IconHabits.svelte';
import IconPanel from '../icons/IconPanel.svelte';
import IconPasswords from '../icons/IconPasswords.svelte';
import IconSettings from '../icons/IconSettings.svelte';

/**
 * Everything that can be opened.
 *
 * A closed set rather than a string, so a section nobody designed cannot be routed to and
 * a typo is a compile error rather than a blank screen.
 */
export type SectionId = 'panel' | 'habits' | 'passwords' | 'finances' | 'settings';

/** How a section presents itself everywhere it appears. */
export interface Section {
  readonly id: SectionId;
  /** What it is called on screen, in Spanish like the rest of the interface. */
  readonly title: string;
  /** The keys that open it, written the way they are drawn. */
  readonly shortcut: string;
  /**
   * The class that carries this section's three colours.
   *
   * A class and not three token references, because the content security policy forbids
   * inline styles and a `style` attribute is how those references used to reach the
   * component. The class is declared once in `base.css`; see the note there.
   */
  readonly tone: string;
  /** Its icon, in the menu and the palette. */
  readonly icon: Component<{ label?: string | undefined }>;
  /** Whether its tab can be closed. The panel's cannot. */
  readonly closable: boolean;
}

/**
 * The five, in the order they are read.
 *
 * Settings borrows the passwords colour rather than having one of its own. It is not a
 * module and has no data behind it, and inventing a fifth hue would mean one more colour
 * carrying a meaning that is only "this is not one of the three".
 */
export const SECTIONS: readonly Section[] = [
  {
    id: 'panel',
    title: 'Panel',
    shortcut: 'Ctrl 0',
    tone: 'tone-panel',
    icon: IconPanel,
    closable: false,
  },
  {
    id: 'habits',
    title: 'Hábitos',
    shortcut: 'Ctrl 1',
    tone: 'tone-habits',
    icon: IconHabits,
    closable: true,
  },
  {
    id: 'passwords',
    title: 'Contraseñas',
    shortcut: 'Ctrl 2',
    tone: 'tone-passwords',
    icon: IconPasswords,
    closable: true,
  },
  {
    id: 'finances',
    title: 'Finanzas',
    shortcut: 'Ctrl 3',
    tone: 'tone-finances',
    icon: IconFinances,
    closable: true,
  },
  {
    id: 'settings',
    title: 'Ajustes',
    shortcut: 'Ctrl ,',
    tone: 'tone-settings',
    icon: IconSettings,
    closable: true,
  },
];

/** The three sections a number key opens, in the order the numbers go. */
export const NUMBERED: readonly SectionId[] = ['panel', 'habits', 'passwords', 'finances'];

/**
 * How a section presents itself.
 *
 * Total over `SectionId`, so there is no "not found" for a caller to handle: the type is
 * what guarantees the answer exists.
 */
export function sectionOf(id: SectionId): Section {
  const found = SECTIONS.find((section) => section.id === id);
  if (found === undefined) {
    // Unreachable while `SectionId` and `SECTIONS` agree, and this is what makes the
    // return type honest instead of optional. If it ever fires, the list lost an entry.
    throw new Error(`la sección «${id}» no está en el catálogo`);
  }
  return found;
}
