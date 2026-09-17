<script lang="ts">
  /**
   * Finances, before there is any movement.
   *
   * The summary is drawn at zero, and zero is not an invented number: a vault with nothing
   * in it has earned nothing, spent nothing and holds nothing. That is the difference
   * between this screen and a mock-up, and it is why the figures are here rather than
   * replaced by dashes — the shape of the screen is three totals and a list, and somebody
   * has to be able to see that before deciding it is what they wanted.
   *
   * The euro is written in because there has to be a unit for the amounts to read as
   * amounts. Which currencies exist and how one is chosen is the finances phase's decision,
   * along with the rule that money is an integer of minor units and never a float.
   */
  import Badge from '../../lib/shell/Badge.svelte';
  import EmptyState from '../../lib/shell/EmptyState.svelte';
  import ScreenHeader from '../../lib/shell/ScreenHeader.svelte';
  import { sectionOf } from '../../lib/shell/sections';

  const section = sectionOf('finances');

  /** The three totals of a month, all of them nothing. */
  const TOTALS = [
    { title: 'Ingresos', amount: '0,00 €' },
    { title: 'Gastos', amount: '0,00 €' },
    { title: 'Saldo', amount: '0,00 €' },
  ];
</script>

<ScreenHeader
  {section}
  title="Finanzas"
  lede="Lo que entra y lo que sale, mes a mes. Nada se ha registrado todavía: esto es la forma que va a tener."
/>

<section class="summary {section.tone}">
  <h2>Este mes</h2>

  <dl>
    {#each TOTALS as total (total.title)}
      <div class="total">
        <dt class="label">{total.title}</dt>
        <dd>{total.amount}</dd>
      </div>
    {/each}
  </dl>

  <div class="foot">
    <p>A cero porque no hay ningún movimiento, no porque el cálculo esté pendiente.</p>
    <Badge tone={section.tone} text="En desarrollo" />
  </div>
</section>

<section class="list">
  <h2>Movimientos</h2>

  <EmptyState
    {section}
    sentence="Todavía no tienes ningún movimiento."
    action="Añadir movimiento"
    note="Registrar movimientos llega en la fase de este módulo. Hasta entonces la pantalla enseña su forma, no sus datos."
  />
</section>

<style>
  section {
    margin-top: var(--space-7);
  }

  h2 {
    margin: 0 0 var(--space-4);
    font-size: var(--text-lg);
    letter-spacing: normal;
  }

  dl {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(var(--card-min-width), 1fr));
    gap: var(--space-4);
    margin: 0;
  }

  /* A raised surface with a border and the module's edge on top, like every other card in
   * this application. No shadow, because there is no elevation scale. */
  .total {
    padding: var(--space-5);
    border: var(--border-width) solid var(--colour-border);
    border-top: var(--card-edge-width) solid var(--tone-colour);
    border-radius: 0 0 var(--radius-md) var(--radius-md);
    background-color: var(--colour-surface-raised);
  }

  dt {
    color: var(--colour-text-muted);
  }

  dd {
    margin: var(--space-3) 0 0;
    font-family: var(--font-mono);
    font-size: var(--text-2xl);
    letter-spacing: var(--tracking-tight);
  }

  .foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-4);
  }

  .foot p {
    max-width: var(--measure);
    margin: 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .list {
    border-top: var(--border-width) solid var(--colour-border);
  }

  .list h2 {
    margin-top: var(--space-6);
  }
</style>
