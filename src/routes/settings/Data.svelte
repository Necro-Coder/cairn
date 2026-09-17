<script lang="ts">
  /**
   * Copies, exporting and importing — declared, and every one of them switched off.
   *
   * They are drawn now rather than left out, because the question "can I get my data out of
   * this?" is one somebody asks before they put anything in, and a settings screen with no
   * answer reads as "no". This says what each one will do and why it cannot do it yet.
   *
   * The design system's rule about disabled controls is the whole point of this screen: a
   * control that is dead for an unexplained reason is a bug report waiting to be filed, so
   * each of these carries its reason in its `title` and in visible text beside it.
   */
  import Badge from '../../lib/shell/Badge.svelte';
  import { sectionOf } from '../../lib/shell/sections';

  /** Borrowed from settings' own colour, because none of this belongs to a module. */
  const section = sectionOf('settings');

  const OPERATIONS = [
    {
      title: 'Exportar una copia',
      detail:
        'Un fichero cifrado con todo lo guardado, para llevártelo o para guardarlo aparte. Llega en la fase de importación y exportación.',
    },
    {
      title: 'Importar una copia',
      detail:
        'Leer un fichero exportado desde este u otro equipo. Llega con la exportación, y por el mismo motivo: sin las dos, una de ellas es una trampa.',
    },
    {
      title: 'Copia de seguridad de la cabecera',
      detail:
        'La cabecera es lo único sin lo cual no se puede abrir nada, y hoy se copia a mano desde la carpeta de la aplicación. Aquí habrá un botón.',
    },
  ];
</script>

<section>
  <h2>Copias, exportar e importar</h2>

  <p class="muted">
    Todavía no hay base de datos, así que no hay nada que copiar ni que exportar. Lo que habrá está
    aquí escrito para que se vea qué falta, no para que parezca que existe.
  </p>

  <ul>
    {#each OPERATIONS as operation (operation.title)}
      <li>
        <div class="head">
          <button type="button" disabled title="Todavía no: {operation.detail}">
            {operation.title}
          </button>
          <Badge colour={section.colour} tint={section.tint} text="En desarrollo" />
        </div>
        <p>{operation.detail}</p>
      </li>
    {/each}
  </ul>
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

  ul {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* Separated by a rule rather than by alternating background, like every list here. */
  li {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4) 0;
    border-top: var(--border-width) solid var(--colour-border);
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  button {
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font-size: var(--text-sm);
  }

  button:disabled {
    border-color: var(--colour-border);
    color: var(--colour-text-faint);
  }

  p {
    max-width: var(--measure);
    margin: 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .muted {
    max-width: var(--measure);
    color: var(--colour-text-muted);
  }
</style>
