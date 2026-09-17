<script lang="ts">
  /**
   * The panel's contents: whatever cards somebody put there, and the way to put more.
   *
   * It starts empty, and that is its normal shape rather than a fault. A panel that arrived
   * full of defaults would be a panel full of somebody else's idea of what matters, and the
   * first thing anybody did with it would be to take things off.
   *
   * There is no geometric composition in the empty state, although the design system allows
   * an empty state one. The panel header above already carries the reduced composition, and
   * two of them on one screen is the thing the budget in that table exists to prevent.
   *
   * Nothing here is written down anywhere. The cards live in the workspace and go with it
   * when the vault closes; keeping them between runs needs an encrypted place to keep them
   * and arrives with phase 03.
   */
  import CardPicker from './CardPicker.svelte';
  import PanelCard from './PanelCard.svelte';
  import { cardOf, type CardId } from './cards';
  import { workspace } from '../workspace.svelte';

  const chosen = $derived(workspace.state?.cards ?? []);

  function toggle(id: CardId): void {
    if (chosen.includes(id)) {
      workspace.removeCard(id);
      return;
    }
    workspace.addCard(id);
  }
</script>

<section class="board">
  {#if chosen.length === 0}
    <p class="empty">
      Tu panel está vacío. Elige las tarjetas que quieras ver al abrir la caja fuerte; las declara
      cada módulo y puedes quitarlas cuando quieras.
    </p>
  {:else}
    <div class="cards">
      {#each chosen as id (id)}
        <PanelCard card={cardOf(id)} onremove={() => workspace.removeCard(id)} />
      {/each}
    </div>
  {/if}

  <CardPicker {chosen} ontoggle={toggle} />
</section>

<style>
  .board {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-5);
    margin-top: var(--space-7);
  }

  .empty {
    max-width: var(--measure);
    color: var(--colour-text-muted);
    font-size: var(--text-lg);
  }

  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--card-min-width), 1fr));
    gap: var(--space-4);
    width: 100%;
  }
</style>
