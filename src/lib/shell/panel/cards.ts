/**
 * The cards the three modules offer to the panel.
 *
 * The panel is composed rather than designed: it starts empty and holds whatever somebody
 * put in it. This is the catalogue they choose from, declared in one place so that a module
 * adding a card does not have to be wired into the panel as well.
 *
 * **A card never shows a value.** Not a balance, not a password, not whether a habit was
 * done. The panel is the first thing on screen after unlocking and it is what is visible to
 * anybody who walks past a window somebody stepped away from. A card shows what it is and,
 * until its module is written, that it is in development. The rule is the same one behind
 * the search contract, for the same reason, and it is easier to keep here because the
 * catalogue is a list of titles.
 */

import type { ModuleId } from '../../search/contract';

/** Everything that can be put on the panel. */
export type CardId =
  | 'habits-today'
  | 'habits-streaks'
  | 'passwords-recent'
  | 'passwords-review'
  | 'finances-month'
  | 'finances-latest';

/** One entry in the catalogue. */
export interface PanelCard {
  readonly id: CardId;
  /** Which module declares it, which is where its colour and its icon come from. */
  readonly module: ModuleId;
  /** What it is called, on the card and in the picker. */
  readonly title: string;
  /** What will be here once the module is written. One sentence, in the future tense. */
  readonly lede: string;
}

/** The six, grouped by the module that declares them. */
export const CARDS: readonly PanelCard[] = [
  {
    id: 'habits-today',
    module: 'habits',
    title: 'Hoy',
    lede: 'Los hábitos que tocan hoy, para marcarlos sin salir del panel.',
  },
  {
    id: 'habits-streaks',
    module: 'habits',
    title: 'Rachas',
    lede: 'Cuántos días seguidos llevas con cada hábito.',
  },
  {
    id: 'passwords-recent',
    module: 'passwords',
    title: 'Añadidas hace poco',
    lede: 'Las últimas cuentas que guardaste, por su nombre.',
  },
  {
    id: 'passwords-review',
    module: 'passwords',
    title: 'Pendientes de revisar',
    lede: 'Las contraseñas repetidas o antiguas que conviene cambiar.',
  },
  {
    id: 'finances-month',
    module: 'finances',
    title: 'Resumen del mes',
    lede: 'En qué se ha ido el mes, por categorías.',
  },
  {
    id: 'finances-latest',
    module: 'finances',
    title: 'Últimos movimientos',
    lede: 'Los movimientos más recientes, por su concepto.',
  },
];

/**
 * What a card is.
 *
 * Total over `CardId`, like `sectionOf`: the type is what guarantees there is an answer, so
 * no caller has a "not found" branch to write and get wrong.
 */
export function cardOf(id: CardId): PanelCard {
  const found = CARDS.find((card) => card.id === id);
  if (found === undefined) {
    // Unreachable while `CardId` and `CARDS` agree. If it ever fires, the list lost an entry.
    throw new Error(`la tarjeta «${id}» no está en el catálogo`);
  }
  return found;
}
