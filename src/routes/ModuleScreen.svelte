<script lang="ts">
  /**
   * What a module looks like before the module exists.
   *
   * Habits, passwords and finances all have a tab, a header and a colour, and nothing
   * behind them yet. This is what they draw until they do: their own empty state, and a
   * badge saying the part is in development.
   *
   * Both, rather than one or the other. The empty state is the real thing somebody will
   * see on their first day with a working module, so drawing it now is drawing the real
   * screen; the badge is what stops that from being a lie about what works today.
   *
   * The action is shown and it reacts. A button that does nothing looks like a broken
   * application; one that explains looks like an unfinished one, which is what this is.
   */
  import Marks from '../lib/shell/Marks.svelte';
  import ScreenHeader from '../lib/shell/ScreenHeader.svelte';
  import type { Section } from '../lib/shell/sections';

  interface Props {
    /** Which module this is. */
    section: Section;
    /** What the person does not have any of yet. */
    empty: string;
    /** What the button that will one day create one says. */
    action: string;
  }

  const { section, empty, action }: Props = $props();

  let explained = $state(false);
</script>

<ScreenHeader {section} title={section.title} />

<div class="empty" style="--module-colour: {section.colour}; --module-tint: {section.tint}">
  <Marks />

  <p>{empty}</p>

  <div class="actions">
    <button type="button" class="primary" onclick={() => (explained = true)}>{action}</button>

    {#if explained}
      <span class="badge">En desarrollo · llega en la fase de este módulo</span>
    {/if}
  </div>
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-5);
    margin-top: var(--space-7);
  }

  .empty p {
    font-size: var(--text-lg);
    color: var(--colour-text-muted);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .primary {
    padding: var(--space-3) var(--space-4);
    border: 0;
    border-radius: var(--radius-sm);
    background-color: var(--colour-accent);
    color: var(--colour-accent-contrast);
    font-weight: var(--weight-semibold);
    transition: background-color var(--duration-fast) var(--easing);
  }

  .primary:hover {
    background-color: var(--colour-accent-strong);
  }

  .badge {
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background-color: var(--module-tint);
    color: var(--module-colour);
    font-size: var(--text-label);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
  }
</style>
