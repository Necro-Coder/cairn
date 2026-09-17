<script lang="ts">
  /**
   * The panel: the first tab, and what appears on opening the vault.
   *
   * Empty until somebody puts a card in it, and that is its normal shape rather than a
   * fault.
   */
  import PanelBoard from '../lib/shell/panel/PanelBoard.svelte';
  import ScreenHeader from '../lib/shell/ScreenHeader.svelte';
  import { sectionOf } from '../lib/shell/sections';

  const section = sectionOf('panel');
</script>

<ScreenHeader
  {section}
  title="Tu panel"
  lede="Las tarjetas las eliges tú. El color de cada una dice de qué sección viene, y aquí solo aparece lo que hayas puesto."
  marks
/>

<PanelBoard />
