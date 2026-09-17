<script lang="ts">
  /**
   * The panel: the first tab, and what appears on opening the vault.
   *
   * Empty until somebody puts a card in it, and that is its normal shape rather than a
   * fault.
   */
  import BackupReminder from '../lib/shell/BackupReminder.svelte';
  import PanelBoard from '../lib/shell/panel/PanelBoard.svelte';
  import ScreenHeader from '../lib/shell/ScreenHeader.svelte';
  import { sectionOf } from '../lib/shell/sections';
  import { workspace } from '../lib/shell/workspace.svelte';

  const section = sectionOf('panel');

  /**
   * Takes somebody from the reminder to the place a backup is made.
   *
   * A temporary tab, like every other place the application opens something on somebody's
   * behalf: they asked to make a copy, not to rearrange their tabs.
   */
  function openData(): void {
    workspace.open('settings', { temporary: true });
    workspace.openSettings('data');
  }
</script>

<ScreenHeader
  {section}
  title="Tu panel"
  lede="Las tarjetas las eliges tú. El color de cada una dice de qué sección viene, y aquí solo aparece lo que hayas puesto."
  marks
/>

<BackupReminder onopen={openData} />

<PanelBoard />
