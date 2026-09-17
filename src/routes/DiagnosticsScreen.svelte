<script lang="ts">
  /**
   * The diagnostics, on a screen of their own, for when the vault is closed.
   *
   * With the vault open this lives inside the settings screen, as its fourth part. This is
   * the other half of the same decision rather than a leftover: the machine somebody needs
   * diagnostics on is usually the one that will not open, and a version number that can only
   * be read after typing the password is a version number nobody can read when it matters.
   *
   * It is a screen and not a layer. The panel it replaces was drawn above everything else,
   * which is how it survived a lock in PR #31 and how the lock screen ended up underneath it.
   * This is only reachable while the vault is closed, so there is no lock for it to survive.
   */
  import Diagnostics from './settings/Diagnostics.svelte';
  import type { StartupResult } from '../lib/startup';

  interface Props {
    /** What the startup measurement found, if it has run. */
    startup: StartupResult | null;
    /** Goes back to whatever the vault's state says should be on screen. */
    onclose: () => void;
  }

  const { startup, onclose }: Props = $props();
</script>

<section class="screen">
  <header>
    <div>
      <span class="label">Diagnóstico</span>
      <h1>Qué es esta copia</h1>
      <div class="rule" aria-hidden="true"></div>
    </div>
    <button type="button" class="close" onclick={onclose}>Volver</button>
  </header>

  <Diagnostics {startup} />
</section>

<style>
  .screen {
    display: flex;
    max-width: var(--content-max);
    flex-direction: column;
    gap: var(--space-6);
    margin: 0 auto;
    width: 100%;
  }

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
  }

  header > div {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
  }

  h1 {
    margin-top: var(--space-3);
  }

  .rule {
    width: var(--space-8);
    height: var(--rule-width);
    margin-top: var(--space-4);
    background-color: var(--colour-rule);
  }

  .close {
    flex: none;
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font-size: var(--text-sm);
    transition: border-color var(--duration-fast) var(--easing);
  }

  .close:hover {
    border-color: var(--colour-accent);
  }
</style>
