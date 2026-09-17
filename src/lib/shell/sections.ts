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
   * The colour this section is written in, as a token reference.
   *
   * A reference rather than a value: every colour in the interface comes from
   * `tokens.css`, and the gate that enforces that reads this file too.
   */
  readonly colour: string;
  /** The tint it fills with: the active tab's background, a chosen row. */
  readonly tint: string;
  /** The colour its figure is drawn in, which for finances is not the colour above. */
  readonly mark: string;
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
    colour: 'var(--module-panel)',
    tint: 'var(--module-panel-tint)',
    mark: 'var(--module-panel-mark)',
    icon: IconPanel,
    closable: false,
  },
  {
    id: 'habits',
    title: 'Hábitos',
    shortcut: 'Ctrl 1',
    colour: 'var(--module-habits)',
    tint: 'var(--module-habits-tint)',
    mark: 'var(--module-habits-mark)',
    icon: IconHabits,
    closable: true,
  },
  {
    id: 'passwords',
    title: 'Contraseñas',
    shortcut: 'Ctrl 2',
    colour: 'var(--module-passwords)',
    tint: 'var(--module-passwords-tint)',
    mark: 'var(--module-passwords-mark)',
    icon: IconPasswords,
    closable: true,
  },
  {
    id: 'finances',
    title: 'Finanzas',
    shortcut: 'Ctrl 3',
    colour: 'var(--module-finances)',
    tint: 'var(--module-finances-tint)',
    mark: 'var(--module-finances-mark)',
    icon: IconFinances,
    closable: true,
  },
  {
    id: 'settings',
    title: 'Ajustes',
    shortcut: 'Ctrl ,',
    colour: 'var(--module-passwords)',
    tint: 'var(--module-passwords-tint)',
    mark: 'var(--module-passwords-mark)',
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
