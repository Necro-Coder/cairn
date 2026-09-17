<script lang="ts">
  /**
   * The `Abrir` menu: the one place where everything that can be opened is listed.
   *
   * A real menu rather than a list of links, because it has to be usable with the keyboard
   * alone: the button opens it, the arrows walk it, `Home` and `End` jump, `Escape` closes
   * it and puts the focus back where it came from, and `Tab` closes it rather than walking
   * off into a popup the eye has already left.
   */
  import type { Snippet } from 'svelte';

  import IconChevron from '../icons/IconChevron.svelte';
  import { SECTIONS, type SectionId } from './sections';

  interface Props {
    /** Called with whatever was chosen. */
    onchoose: (id: SectionId) => void;
    /** Anything the menu offers besides the sections, drawn under a separator. */
    extra?: Snippet<[() => void]> | undefined;
  }

  const { onchoose, extra }: Props = $props();

  let open = $state(false);
  let trigger = $state<HTMLButtonElement | null>(null);
  let list = $state<HTMLDivElement | null>(null);

  /** Every item currently in the menu, in the order they are drawn. */
  function items(): HTMLElement[] {
    return [...(list?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
  }

  function focusItem(index: number): void {
    const all = items();
    if (all.length === 0) {
      return;
    }
    // Wraps at both ends, which is what a menu does and what somebody holding an arrow key
    // expects. The modulo is written twice because -1 % n is -1 in JavaScript.
    all[((index % all.length) + all.length) % all.length]?.focus();
  }

  function show(): void {
    open = true;
    // After the menu exists. Reading the list in the same tick would find nothing, because
    // it has not been drawn yet.
    queueMicrotask(() => focusItem(0));
  }

  function hide(options: { readonly restoreFocus: boolean }): void {
    open = false;
    if (options.restoreFocus) {
      trigger?.focus();
    }
  }

  function toggle(): void {
    if (open) {
      hide({ restoreFocus: true });
    } else {
      show();
    }
  }

  function choose(id: SectionId): void {
    hide({ restoreFocus: false });
    onchoose(id);
  }

  function onMenuKeydown(event: KeyboardEvent): void {
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
        // Not prevented: leaving with Tab is legitimate, and the menu closes behind.
        hide({ restoreFocus: false });
        break;
      default:
        break;
    }
  }

  /**
   * Closes the menu when the focus leaves it altogether.
   *
   * `relatedTarget` is where the focus went. Inside the menu means moving between items;
   * null means the window lost focus, and the menu should not be left open over a window
   * somebody has walked away from.
   */
  function onFocusOut(event: FocusEvent): void {
    const goingTo = event.relatedTarget;
    if (goingTo instanceof Node && (list?.contains(goingTo) === true || goingTo === trigger)) {
      return;
    }
    open = false;
  }
</script>

<!-- Marked so a press here does not also hand the window to the window manager. -->
<div class="open-menu" data-no-drag>
  <button
    bind:this={trigger}
    type="button"
    class="trigger"
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={toggle}
    onkeydown={(event) => {
      if (event.key === 'ArrowDown' && !open) {
        event.preventDefault();
        show();
      }
    }}
  >
    Abrir
    <IconChevron />
  </button>

  {#if open}
    <div
      bind:this={list}
      class="list"
      role="menu"
      aria-label="Abrir"
      tabindex="-1"
      onkeydown={onMenuKeydown}
      onfocusout={onFocusOut}
    >
      <span class="label heading">Abrir</span>

      {#each SECTIONS as section (section.id)}
        {@const Icon = section.icon}
        <button
          type="button"
          role="menuitem"
          class="item {section.tone}"
          onclick={() => choose(section.id)}
        >
          <span class="chip mark" aria-hidden="true"></span>
          <Icon />
          {section.title}
          <span class="shortcut">{section.shortcut}</span>
        </button>
      {/each}

      {#if extra !== undefined}
        <div class="separator"></div>
        {@render extra(() => hide({ restoreFocus: false }))}
      {/if}
    </div>
  {/if}
</div>

<style>
  .open-menu {
    position: relative;
  }

  .trigger {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: var(--colour-text);
    font-size: var(--text-sm);
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
    width: var(--layer-width-menu);
    padding: var(--space-1);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
  }

  .heading {
    padding: var(--space-2) var(--space-3) var(--space-1);
  }

  .item {
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

  .item:hover {
    background-color: var(--colour-surface-sunken);
  }

  .chip {
    width: var(--chip-size);
    height: var(--chip-size);
    flex: none;
    background-color: var(--tone-mark);
  }

  .shortcut {
    margin-left: auto;
    color: var(--colour-text-faint);
    font-size: var(--text-xs);
  }

  .separator {
    height: var(--border-width);
    margin: var(--space-1) 0;
    background-color: var(--colour-border);
  }
</style>
