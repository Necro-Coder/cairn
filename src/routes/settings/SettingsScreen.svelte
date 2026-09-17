<script lang="ts">
  /**
   * Settings, in five parts.
   *
   * One at a time rather than one long page, because four of the five are things somebody
   * came here to do and the fifth is a table they read once; scrolling past the master
   * password to reach the shortcuts would be the wrong shape for both.
   *
   * The chooser is a row of ordinary buttons with `aria-pressed`, the same pattern as the
   * inactivity periods below it, rather than the ARIA tab pattern. The window already has
   * tabs, a second row of them would be two things called the same thing, and ARIA tabs done
   * without their roving focus are worse than buttons that simply work.
   *
   * Which part is open lives in the workspace, so that `Ctrl` `⇧` `D` can open this screen
   * straight at the diagnostics, and so that it goes away with everything else when the vault
   * closes.
   */
  import Appearance from './Appearance.svelte';
  import Data from './Data.svelte';
  import Diagnostics from './Diagnostics.svelte';
  import Security from './Security.svelte';
  import Shortcuts from './Shortcuts.svelte';
  import ScreenHeader from '../../lib/shell/ScreenHeader.svelte';
  import { SETTINGS_SECTIONS, settingsSectionOf } from './sections';
  import { sectionOf } from '../../lib/shell/sections';
  import { workspace } from '../../lib/shell/workspace.svelte';
  import type { StartupResult } from '../../lib/startup';

  interface Props {
    /** What the startup measurement found, for the diagnostics part. */
    startup: StartupResult | null;
  }

  const { startup }: Props = $props();

  const section = sectionOf('settings');
  const open = $derived(settingsSectionOf(workspace.state?.settings ?? 'security'));
</script>

<ScreenHeader {section} title="Ajustes" lede={open.lede} />

<nav class="parts" aria-label="Apartados de ajustes">
  {#each SETTINGS_SECTIONS as part (part.id)}
    <button
      type="button"
      class:chosen={part.id === open.id}
      aria-pressed={part.id === open.id}
      onclick={() => workspace.openSettings(part.id)}
    >
      {part.title}
    </button>
  {/each}
</nav>

<div class="body">
  {#if open.id === 'security'}
    <Security />
  {:else if open.id === 'appearance'}
    <Appearance />
  {:else if open.id === 'data'}
    <Data />
  {:else if open.id === 'diagnostics'}
    <Diagnostics {startup} />
  {:else}
    <Shortcuts />
  {/if}
</div>

<style>
  .parts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: var(--space-6);
  }

  .parts button {
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    /* Reserved whether or not it is drawn, so choosing does not move the words. */
    border-bottom: var(--rule-width) solid transparent;
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  /*
   * The chosen one carries an edge, a surface and a weight, not a vermilion fill. This is
   * navigation inside a screen and the strip above it says so the same way; and the parts
   * below already spend the one accented fill a screen is allowed on the value they have
   * chosen. Three shades of vermilion in one view and the rule that vermilion means action
   * is worth nothing.
   */
  .parts button.chosen {
    border-color: var(--colour-border-strong);
    border-bottom-color: var(--colour-accent);
    background-color: var(--colour-surface-sunken);
    color: var(--colour-text);
    font-weight: var(--weight-semibold);
  }

  .body {
    margin-top: var(--space-6);
    padding-top: var(--space-5);
    border-top: var(--border-width) solid var(--colour-border);
  }
</style>
