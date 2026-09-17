<script lang="ts">
  /**
   * One card on the panel.
   *
   * It shows what it is and what will be in it, and it shows no value. Not a balance, not a
   * user name, not whether a habit was done today. The panel is the first thing on screen
   * after unlocking and the thing visible to anybody who walks past a window somebody
   * stepped away from, and a card that showed a number would put that number there for the
   * rest of the afternoon.
   *
   * Until the modules are written, the place the value will go holds the badge that says so.
   */
  import Badge from '../Badge.svelte';
  import IconClose from '../../icons/IconClose.svelte';
  import { sectionOf } from '../sections';
  import type { PanelCard } from './cards';

  interface Props {
    /** What this card is. */
    card: PanelCard;
    /** Takes it off the panel. */
    onremove: () => void;
  }

  const { card, onremove }: Props = $props();

  const section = $derived(sectionOf(card.module));
</script>

<article class="card {section.tone}">
  <header>
    <div>
      <span class="label">{section.title}</span>
      <h2>{card.title}</h2>
    </div>

    <button type="button" class="remove" title="Quitar «{card.title}» del panel" onclick={onremove}>
      <IconClose label="Quitar «{card.title}» del panel" />
    </button>
  </header>

  <p>{card.lede}</p>

  <!-- Where the value will go, which is why the badge is at the bottom and not beside the
       title: it is standing in for something, not labelling the card. -->
  <Badge tone={section.tone} text="En desarrollo" />
</article>

<style>
  .card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
    min-height: var(--card-min-height);
    padding: var(--space-5);
    border: var(--border-width) solid var(--colour-border);
    /* Square at the top, where the module's edge is, and eased at the bottom. */
    border-top: var(--card-edge-width) solid var(--tone-colour);
    border-radius: 0 0 var(--radius-md) var(--radius-md);
    background-color: var(--colour-surface-raised);
  }

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    width: 100%;
  }

  h2 {
    margin-top: var(--space-1);
    font-size: var(--text-lg);
  }

  p {
    flex: 1;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .remove {
    display: grid;
    flex: none;
    place-items: center;
    width: var(--control-min-size);
    height: var(--control-min-size);
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: var(--colour-text-faint);
    transition: background-color var(--duration-fast) var(--easing);
  }

  .remove:hover {
    background-color: var(--colour-surface-sunken);
    color: var(--colour-text);
  }
</style>
