<script lang="ts">
  /**
   * Passwords, before there are any.
   *
   * The search field is drawn and switched off, which is the one control on these three
   * screens that cannot follow the rule about pressing a dead button and being told why: a
   * field is not pressed, it is typed into, and a field that accepted text and answered
   * nothing would be worse than one that plainly cannot be used. So it carries its reason
   * in its `title` and again in visible text, which is what the design system asks of any
   * disabled control. It carries no "in development" badge of its own: the badge marks where
   * a value will appear, and the only place a search on this screen would put one is the
   * list underneath, which already has it.
   *
   * Nothing on this screen shows a value, and on this one that is not a matter of there
   * being no data yet. It is the rule that governs the module: a list of accounts shows the
   * name of the site and never what is behind it, and it is written here now so that the
   * phase which fills the list inherits a screen that never had a column for it.
   */
  import EmptyState from '../../lib/shell/EmptyState.svelte';
  import ScreenHeader from '../../lib/shell/ScreenHeader.svelte';
  import { sectionOf } from '../../lib/shell/sections';

  const section = sectionOf('passwords');

  /** Said twice, in the `title` and on screen, so it reaches the pointer and the eye. */
  const WHY_DISABLED = 'Buscar entre las contraseñas llega en la fase de este módulo.';
</script>

<ScreenHeader
  {section}
  title="Contraseñas"
  lede="Tus cuentas, guardadas bajo la misma llave que todo lo demás. Aquí se ve el nombre del sitio; lo de dentro se pide."
/>

<section class="find">
  <label class="label" for="passwords-search">Buscar</label>

  <input id="passwords-search" type="search" disabled title="Todavía no: {WHY_DISABLED}" />

  <p class="why">{WHY_DISABLED}</p>
</section>

<section class="list">
  <h2>Tus contraseñas</h2>

  <EmptyState
    {section}
    sentence="Todavía no tienes ninguna contraseña guardada."
    action="Añadir contraseña"
    note="Guardar y leer contraseñas llega en la fase de este módulo. Hasta entonces la pantalla enseña su forma, no sus datos."
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

  .find {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
  }

  input {
    width: 100%;
    max-width: var(--field-max-width);
    padding: var(--space-3);
    border: var(--border-width) solid var(--colour-border);
    border-radius: var(--radius-sm);
    background-color: var(--colour-surface-sunken);
    color: var(--colour-text);
    font-family: var(--font-sans);
    font-size: var(--text-base);
  }

  input:disabled {
    color: var(--colour-text-faint);
  }

  .why {
    max-width: var(--measure);
    margin: var(--space-2) 0 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .list {
    margin-top: var(--space-6);
    border-top: var(--border-width) solid var(--colour-border);
  }

  .list h2 {
    margin-top: var(--space-6);
  }
</style>
