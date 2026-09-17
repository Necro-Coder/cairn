<script lang="ts">
  /**
   * Settings.
   *
   * Today it is the security section and nothing else: the master password, the derivation
   * parameters and the inactivity period, which used to live inside the diagnostics panel.
   * The other four sections — appearance, data, diagnostics and shortcuts — arrive with the
   * step of this phase that restyles the existing screens.
   */
  import ScreenHeader from '../lib/shell/ScreenHeader.svelte';
  import { sectionOf } from '../lib/shell/sections';
  import VaultPanel from './VaultPanel.svelte';

  const section = sectionOf('settings');
</script>

<ScreenHeader {section} title="Ajustes" lede="Seguridad de la caja fuerte." />

<div class="body">
  <VaultPanel />
</div>

<style>
  .body {
    margin-top: var(--space-6);
  }
</style>
