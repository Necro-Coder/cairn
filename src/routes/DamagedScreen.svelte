<script lang="ts">
  /**
   * A vault that is there and whose header cannot be read.
   *
   * The one thing this screen must never offer is creating a new vault. That would write over
   * the damaged header and make everything encrypted under it unreadable for good, and it is
   * exactly what somebody looking at an application that will not open would press.
   *
   * So there is no action here at all. There is what happened, what it means, and the one
   * file to go and fetch. The warning carries the warning colour and comes before anything
   * else, because a warning about losing data read after the decision is not a warning.
   */
  import Marks from '../lib/shell/Marks.svelte';
</script>

<section class="damaged">
  <div class="column">
    <span class="label">Caja fuerte</span>
    <h1>La cabecera no se puede leer</h1>
    <div class="rule" aria-hidden="true"></div>

    <p>
      Hay una caja fuerte en este equipo y su cabecera está dañada. No se va a crear otra encima:
      eso dejaría ilegible todo lo que haya guardado.
    </p>

    <p class="what-to-do" role="note">
      Recupera el fichero <code>cairn.header</code> desde tu propia copia de seguridad y vuelve a abrir
      la aplicación. Si la aplicación guardó una copia al lado, la usará sola la próxima vez que arranque.
    </p>
  </div>

  <Marks shapes={2} />
</section>

<style>
  .damaged {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-6);
    width: 100%;
    max-width: var(--content-max);
    margin: 0 auto;
  }

  .column {
    display: flex;
    max-width: var(--measure);
    flex-direction: column;
    align-items: flex-start;
  }

  h1 {
    margin-top: var(--space-3);
    color: var(--colour-negative);
  }

  .rule {
    width: var(--space-8);
    height: var(--rule-width);
    margin-top: var(--space-4);
    background-color: var(--colour-negative);
  }

  p {
    margin-top: var(--space-4);
  }

  .what-to-do {
    padding: var(--space-4);
    border: var(--border-width) solid var(--colour-warning);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
  }
</style>
