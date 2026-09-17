<script lang="ts">
  /**
   * The application frame: the header, and whatever section is open under it.
   *
   * Everything here is drawn only while the vault is open. The screens outside it — the
   * lock screen, creating the vault, a header that cannot be read — have no shell, because
   * the shell is the part that offers a way into data.
   *
   * The keyboard shortcuts live here for the same reason. They open sections, and a
   * section is not something to open when there is nothing to open it onto.
   */
  import ModuleScreen from '../../routes/ModuleScreen.svelte';
  import PanelScreen from '../../routes/PanelScreen.svelte';
  import SettingsScreen from '../../routes/SettingsScreen.svelte';
  import Header from './Header.svelte';
  import { NUMBERED, sectionOf, type SectionId } from './sections';
  import { router } from './router.svelte';

  const current = $derived(router.current);

  /** What each module says when there is nothing in it yet. */
  const MODULE_EMPTY: Record<'habits' | 'passwords' | 'finances', [string, string]> = {
    habits: ['Todavía no tienes ningún hábito.', 'Añadir hábito'],
    passwords: ['Todavía no tienes ninguna contraseña guardada.', 'Añadir contraseña'],
    finances: ['Todavía no tienes ningún movimiento.', 'Añadir movimiento'],
  };

  function open(id: SectionId): void {
    router.open(id);
  }

  /**
   * The shortcuts that move around the application.
   *
   * On the window rather than on a container, because they have to work wherever the focus
   * happens to be. Each one is prevented, so `Ctrl` `0` does not also reset the zoom and
   * `Ctrl` `,` does not type a comma into whatever field is focused.
   */
  function handleKeydown(event: KeyboardEvent): void {
    if (!event.ctrlKey || event.altKey || event.metaKey) {
      return;
    }

    if (event.key === ',') {
      event.preventDefault();
      open('settings');
      return;
    }

    const numbered = NUMBERED[Number(event.key)];
    if (numbered !== undefined && /^[0-9]$/.test(event.key)) {
      event.preventDefault();
      open(numbered);
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<Header onopen={open} />

<main>
  <div class="page">
    {#if current === 'panel'}
      <PanelScreen />
    {:else if current === 'settings'}
      <SettingsScreen />
    {:else}
      {@const [empty, action] = MODULE_EMPTY[current]}
      <ModuleScreen section={sectionOf(current)} {empty} {action} />
    {/if}
  </div>
</main>

<style>
  main {
    flex: 1;
    overflow: auto;
    padding: var(--space-6) var(--space-6) var(--space-8);
  }

  /* Capped and centred in what is left, which is where every screen in the application
   * sits. A screen never sets its own width. */
  .page {
    max-width: var(--content-max);
    margin: 0 auto;
  }
</style>
