<script lang="ts">
  /**
   * Everything that can be done without touching the mouse.
   *
   * Most of it is read from the palette's own catalogue rather than written again here, so a
   * command added later appears on this screen without anybody remembering that this screen
   * exists. What is listed by hand is what is not a command: opening the palette itself, the
   * two gestures on a tab, and the keys that move between tabs.
   *
   * It is a table because it is a table: two columns, one row per shortcut, read down.
   */
  import { COMMANDS } from '../../lib/shell/palette/catalogue';

  /** What the palette can do and what keys do it, for the ones that have keys. */
  const FROM_COMMANDS = COMMANDS.filter((command) => command.shortcut !== null).map((command) => ({
    keys: command.shortcut ?? '',
    what: command.title,
  }));

  /** The rest, which are gestures rather than commands. */
  const OTHERS: readonly { readonly keys: string; readonly what: string }[] = [
    { keys: 'Ctrl K', what: 'Abrir la paleta de comandos' },
    { keys: 'Ctrl ⇧ D', what: 'Ir al diagnóstico' },
    { keys: 'Ctrl Tab', what: 'Ir a la pestaña siguiente' },
    { keys: 'Ctrl ⇧ Tab', what: 'Ir a la pestaña anterior' },
    { keys: '⇧ Intro', what: 'Fijar la pestaña que tienes seleccionada, igual que un doble clic' },
    { keys: '⇧ ← →', what: 'Mover una pestaña por la barra, igual que arrastrarla' },
    { keys: 'Esc', what: 'Cerrar la paleta, un menú o el selector de tarjetas' },
  ];
</script>

<section>
  <h2>Atajos de teclado</h2>

  <p class="muted">
    Todo lo que se hace con el ratón se hace con el teclado. La única excepción es mover la ventana,
    que es un gesto del puntero y no tiene equivalente porque la ventana no lleva la barra del
    sistema.
  </p>

  <table>
    <thead>
      <tr>
        <th scope="col">Teclas</th>
        <th scope="col">Qué hace</th>
      </tr>
    </thead>
    <tbody>
      {#each [...OTHERS, ...FROM_COMMANDS] as shortcut (shortcut.keys)}
        <tr>
          <th scope="row"><kbd>{shortcut.keys}</kbd></th>
          <td>{shortcut.what}</td>
        </tr>
      {/each}
    </tbody>
  </table>
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  h2 {
    margin: 0;
    font-size: var(--text-lg);
    letter-spacing: normal;
  }

  .muted {
    max-width: var(--measure);
    margin: 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    text-align: left;
  }

  th,
  td {
    padding: var(--space-3) var(--space-3) var(--space-3) 0;
    border-bottom: var(--border-width) solid var(--colour-border);
    vertical-align: baseline;
  }

  thead th {
    color: var(--colour-text-muted);
    font-size: var(--text-label);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
  }

  tbody th {
    width: var(--definition-label-width);
    font-weight: var(--weight-regular);
  }

  kbd {
    display: inline-block;
    padding: var(--space-1) var(--space-2);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: var(--colour-surface-sunken);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: nowrap;
  }
</style>
