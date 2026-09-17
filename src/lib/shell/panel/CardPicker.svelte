<script lang="ts">
  /**
   * The list the panel is composed from.
   *
   * Not a dialog. A dialog is for a choice that carrying on cannot undo, and putting a card
   * on the panel is undone by taking it off again — so this is a layer that opens in place,
   * under the button that opened it, and closes on `Escape` with the focus going back where
   * it came from.
   *
   * Each entry is a checkbox rather than an "add" button, so that the list is also the
   * answer to "what is on my panel": the six cards the modules declare, with the ones
   * already there ticked. A separate list of what to remove would be a second place to read
   * the same fact.
   */
  import { CARDS, type CardId } from './cards';
  import { sectionOf } from '../sections';
  import IconPlus from '../../icons/IconPlus.svelte';

  interface Props {
    /** What is on the panel, which is what shows as ticked. */
    chosen: readonly CardId[];
    /** Puts one on or takes it off, depending on where it is now. */
    ontoggle: (id: CardId) => void;
  }

  const { chosen, ontoggle }: Props = $props();

  let open = $state(false);
  let trigger = $state<HTMLButtonElement | null>(null);
  let list = $state<HTMLDivElement | null>(null);

  function items(): HTMLElement[] {
    return [...(list?.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]') ?? [])];
  }

  function focusItem(index: number): void {
    const all = items();
    if (all.length === 0) {
      return;
    }
    // Wraps at both ends. The modulo is written twice because -1 % n is -1 in JavaScript.
    all[((index % all.length) + all.length) % all.length]?.focus();
  }

  function show(): void {
    open = true;
    // After the layer exists: reading the list in the same tick would find nothing.
    queueMicrotask(() => focusItem(0));
  }

  function hide(options: { readonly restoreFocus: boolean }): void {
    open = false;
    if (options.restoreFocus) {
      trigger?.focus();
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    const all = items();
    const here = all.findIndex((item) => item === document.activeElement);

    switch (event.key) {
      case 'Escape':
        event.preventDefault();
        hide({ restoreFocus: true });
        break;
      case 'ArrowDown':
        event.preventDefault();
        focusItem(here + 1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        focusItem(here - 1);
        break;
      case 'Home':
        event.preventDefault();
        focusItem(0);
        break;
      case 'End':
        event.preventDefault();
        focusItem(all.length - 1);
        break;
      case 'Tab':
        // Not prevented: leaving with Tab is legitimate, and the layer closes behind.
        hide({ restoreFocus: false });
        break;
      default:
        break;
    }
  }

  /** Closes the layer when the focus leaves it altogether. */
  function onFocusOut(event: FocusEvent): void {
    const goingTo = event.relatedTarget;
    if (goingTo instanceof Node && (list?.contains(goingTo) === true || goingTo === trigger)) {
      return;
    }
    open = false;
  }
</script>

<div class="picker">
  <button
    bind:this={trigger}
    type="button"
    class="trigger"
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={() => (open ? hide({ restoreFocus: true }) : show())}
  >
    <IconPlus />
    Añadir tarjeta
  </button>

  {#if open}
    <div
      bind:this={list}
      class="list"
      role="menu"
      aria-label="Tarjetas disponibles"
      tabindex="-1"
      onkeydown={onKeydown}
      onfocusout={onFocusOut}
    >
      <span class="label heading">Tarjetas</span>

      {#each CARDS as card (card.id)}
        {@const section = sectionOf(card.module)}
        {@const on = chosen.includes(card.id)}
        <button
          type="button"
          role="menuitemcheckbox"
          aria-checked={on}
          class="item {section.tone}"
          onclick={() => ontoggle(card.id)}
        >
          <span class="chip mark" aria-hidden="true"></span>
          <span class="text">
            <span class="title">{card.title}</span>
            <span class="lede">{section.title} · {card.lede}</span>
          </span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .picker {
    position: relative;
  }

  .trigger {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border: var(--border-width) dashed var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: transparent;
    color: var(--colour-text);
    font-weight: var(--weight-medium);
    transition: background-color var(--duration-fast) var(--easing);
  }

  .trigger:hover {
    background-color: var(--colour-surface-sunken);
  }

  .list {
    position: absolute;
    z-index: 2;
    top: calc(100% + var(--space-2));
    left: 0;
    width: var(--layer-width-picker);
    /* Against the window rather than against the button it hangs from: the button is as
     * wide as its two words, and the layer is a list of sentences. */
    max-width: 90vw;
    max-height: var(--layer-max-height);
    overflow: auto;
    padding: var(--space-1);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
  }

  .heading {
    display: block;
    padding: var(--space-2) var(--space-3) var(--space-1);
  }

  .item {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    border: 0;
    /* Reserved whether or not it is drawn, so ticking a card does not move its text. */
    border-left: var(--rule-width) solid transparent;
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: var(--colour-text);
    text-align: left;
  }

  .item:hover {
    background-color: var(--colour-surface-sunken);
  }

  /* A card already on the panel. The tick is what a screen reader hears; this is what the
   * eye reads, and it is a border rather than a background so it survives forced colours. */
  .item[aria-checked='true'] {
    border-left-color: var(--colour-accent);
  }

  .chip {
    width: var(--chip-size);
    height: var(--chip-size);
    flex: none;
    margin-top: var(--space-1);
    background-color: var(--tone-mark);
  }

  .text {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: var(--space-1);
  }

  .title {
    font-weight: var(--weight-medium);
  }

  .lede {
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }
</style>
