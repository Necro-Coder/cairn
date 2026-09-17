<script lang="ts">
  /**
   * Which of the two themes the application draws in.
   *
   * Three answers, not two. "Automático" follows the system, which is what somebody who has
   * set their machine to go dark in the evening already asked for once and should not have to
   * ask for again; the other two override it.
   *
   * It is written to `data-theme` on the root element, which is exactly what `tokens.css`
   * reads: ink is declared once under `prefers-color-scheme: dark` guarded against an
   * explicit light choice, and once under an explicit dark one. Nothing here knows a colour.
   *
   * It is not remembered between runs. Keeping it would mean writing a preference somewhere,
   * and the only place this application writes is the encrypted vault, which arrives in phase
   * 03. Saying so on the screen is better than a setting that silently forgets.
   */

  /** The three answers, in the order they are offered. */
  const CHOICES = [
    { value: '', label: 'Automático', detail: 'Lo que diga el sistema' },
    { value: 'light', label: 'Papel', detail: 'Claro siempre' },
    { value: 'dark', label: 'Tinta', detail: 'Oscuro siempre' },
  ] as const;

  /**
   * Seeded from the document rather than from a constant.
   *
   * The screen is redrawn whenever its tab is reopened, and reading the element is what makes
   * the chooser agree with what is actually on screen instead of resetting itself.
   */
  let chosen = $state(document.documentElement.dataset['theme'] ?? '');

  function choose(value: string): void {
    chosen = value;
    if (value === '') {
      delete document.documentElement.dataset['theme'];
      return;
    }
    document.documentElement.dataset['theme'] = value;
  }
</script>

<section>
  <h2>Tema</h2>

  <div class="choices" role="group" aria-label="Tema">
    {#each CHOICES as choice (choice.value)}
      <button
        type="button"
        class:chosen={chosen === choice.value}
        aria-pressed={chosen === choice.value}
        onclick={() => choose(choice.value)}
      >
        <span class="label-text">{choice.label}</span>
        <span class="detail">{choice.detail}</span>
      </button>
    {/each}
  </div>

  <p class="muted">
    De momento no se guarda: al cerrar la aplicación vuelve a «Automático». Guardarlo necesita un
    sitio cifrado donde escribirlo, y eso llega con la base de datos.
  </p>
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  h2 {
    margin: 0;
    font-size: var(--text-lg);
    letter-spacing: normal;
  }

  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .choices button {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    text-align: left;
  }

  /* Filled as well as outlined, so colour is never the only signal, and `aria-pressed`
   * carries it for anything that is not looking at the colour at all. */
  .choices button.chosen {
    border-color: var(--colour-accent);
    background-color: var(--colour-accent);
    color: var(--colour-accent-contrast);
  }

  .label-text {
    font-weight: var(--weight-semibold);
  }

  .detail {
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  /* On the filled button the muted grey would be the one thing on this screen that fails
   * contrast, so the second line takes the same colour as the first. */
  .choices button.chosen .detail {
    color: var(--colour-accent-contrast);
  }

  .muted {
    margin: 0;
    max-width: var(--measure);
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }
</style>
