<script lang="ts">
  /**
   * The application frame: the header, the tab strip, and whatever tab is open under them.
   *
   * Everything here is drawn only while the vault is open. The screens outside it — the lock
   * screen, creating the vault, a header that cannot be read — have no shell, because the
   * shell is the part that offers a way into data.
   *
   * The keyboard shortcuts live here for the same reason, and they all go through the
   * workspace rather than changing anything themselves. What opening, closing and pinning
   * mean is in `tabs.ts`, with a test; this decides which keys ask for them.
   */
  import ModuleScreen from '../../routes/ModuleScreen.svelte';
  import PanelScreen from '../../routes/PanelScreen.svelte';
  import SettingsScreen from '../../routes/SettingsScreen.svelte';
  import IconClock from '../icons/IconClock.svelte';
  import { session } from '../session.svelte';
  import CommandPalette from './palette/CommandPalette.svelte';
  import Header from './Header.svelte';
  import TabBar from './TabBar.svelte';
  import { NUMBERED, sectionOf, type SectionId } from './sections';
  import type { Command } from './palette/commands';
  import { workspace } from './workspace.svelte';

  const tabs = $derived(workspace.state?.tabs);
  const current = $derived<SectionId>(
    tabs?.tabs.find((tab) => tab.id === tabs.activeId)?.section ?? 'panel',
  );

  let paletteOpen = $state(false);
  let windowWidth = $state(0);

  // The tab cap is read from the width of the window, and the workspace is where it is read
  // from. Reported here because this is the only component that is always on screen while
  // the vault is open.
  $effect(() => {
    if (windowWidth > 0) {
      workspace.width = windowWidth;
    }
  });

  /** What each module says when there is nothing in it yet. */
  const MODULE_EMPTY: Record<'habits' | 'passwords' | 'finances', [string, string]> = {
    habits: ['Todavía no tienes ningún hábito.', 'Añadir hábito'],
    passwords: ['Todavía no tienes ninguna contraseña guardada.', 'Añadir contraseña'],
    finances: ['Todavía no tienes ningún movimiento.', 'Añadir movimiento'],
  };

  /** Opens a section the way the menu, a shortcut and the palette all open one: temporary. */
  function open(id: SectionId): void {
    workspace.open(id, { temporary: true });
  }

  function closeActiveTab(): void {
    if (tabs !== undefined) {
      workspace.close(tabs.activeId);
    }
  }

  function runCommand(command: Command): void {
    paletteOpen = false;

    switch (command.effect.kind) {
      case 'open':
        open(command.effect.section);
        break;
      case 'closeTab':
        closeActiveTab();
        break;
      case 'reopenTab':
        workspace.reopen();
        break;
      case 'lock':
        void session.lock();
        break;
    }
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

    const key = event.key.toLowerCase();

    if (event.shiftKey) {
      if (key === 't') {
        event.preventDefault();
        workspace.reopen();
      } else if (event.key === 'Tab') {
        event.preventDefault();
        workspace.cycle(-1);
      }
      return;
    }

    switch (key) {
      case 'k':
        event.preventDefault();
        paletteOpen = !paletteOpen;
        return;
      case 'w':
        event.preventDefault();
        closeActiveTab();
        return;
      case ',':
        event.preventDefault();
        open('settings');
        return;
      default:
        break;
    }

    if (event.key === 'Tab') {
      event.preventDefault();
      workspace.cycle(1);
      return;
    }

    const numbered = NUMBERED[Number(event.key)];
    if (numbered !== undefined && /^[0-9]$/.test(event.key)) {
      event.preventDefault();
      open(numbered);
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} bind:innerWidth={windowWidth} />

<Header onopen={open} menuExtra={reopenEntry} />
<TabBar />

{#snippet reopenEntry(close: () => void)}
  <!--
    The keyboard has `Ctrl` `⇧` `T` and the palette has a row for this; the menu is where
    somebody finds it without knowing either. It is disabled when there is nothing to
    reopen, and it says so, because a control that is dead for an unexplained reason is a
    bug report waiting to be filed.
  -->
  {@const nothing = (workspace.state?.tabs.closed.length ?? 0) === 0}
  <button
    type="button"
    role="menuitem"
    class="reopen"
    disabled={nothing}
    title={nothing ? 'No has cerrado ninguna pestaña todavía' : 'Vuelve a abrirla, ya fijada'}
    onclick={() => {
      close();
      workspace.reopen();
    }}
  >
    <IconClock />
    Reabrir la última cerrada
    <span class="shortcut">Ctrl ⇧ T</span>
  </button>
{/snippet}

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

{#if paletteOpen}
  <CommandPalette onrun={runCommand} onclose={() => (paletteOpen = false)} />
{/if}

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

  .reopen {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    border: 0;
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: var(--colour-text);
    text-align: left;
  }

  .reopen:hover:not(:disabled) {
    background-color: var(--colour-surface-sunken);
  }

  .reopen:disabled {
    color: var(--colour-text-faint);
  }

  .shortcut {
    margin-left: auto;
    color: var(--colour-text-faint);
    font-size: var(--text-xs);
  }
</style>
