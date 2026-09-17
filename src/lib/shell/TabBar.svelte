<script lang="ts">
  /**
   * The tab strip, directly under the header and drawn as part of the same band.
   *
   * It holds no rules. Which tab is where, what happens when something is opened and what a
   * double click means are all in `tabs.ts`, over plain values, with a test. This draws the
   * answer and reports what somebody did to it.
   *
   * Two things here are worth knowing before changing it.
   *
   * It is not an ARIA `tablist`. A tab in that pattern is a single control and must not
   * contain another one, and each of these contains a close button. Marking it up as a
   * tablist anyway is the kind of ARIA that reads correctly in a document and behaves worse
   * than plain buttons in a screen reader, so this is what it actually is: a navigation
   * region holding a list of buttons, with `aria-current` on the one being shown.
   *
   * And reordering has a keyboard equivalent. Dragging is a pointer gesture; `Shift` with a
   * left or right arrow on a focused tab moves it the same way. The design system says
   * everything usable with a mouse is usable with a keyboard, and a strip whose order could
   * only be changed by dragging would be the second place in this application to break that
   * promise after the window itself.
   */
  import IconClose from '../icons/IconClose.svelte';
  import { sectionOf } from './sections';
  import { workspace } from './workspace.svelte';

  const tabs = $derived(workspace.state?.tabs.tabs ?? []);
  const activeId = $derived(workspace.state?.tabs.activeId ?? '');

  /** Which tab is being dragged, so the drop knows what to move. */
  let dragging = $state<string | null>(null);

  /**
   * Keeps the focus on the tab that was just moved.
   *
   * Without this, moving a tab with the keyboard moves the button out from under the focus
   * and the next arrow press goes somewhere else entirely. Svelte reuses the elements by
   * key, so the focus has to be put back after the list is redrawn.
   */
  function refocus(id: string): void {
    queueMicrotask(() => {
      document.querySelector<HTMLElement>(`[data-tab="${id}"]`)?.focus();
    });
  }

  function onTitleKeydown(event: KeyboardEvent, id: string, index: number): void {
    if (event.shiftKey && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
      event.preventDefault();
      workspace.move(id, index + (event.key === 'ArrowLeft' ? -1 : 1));
      refocus(id);
      return;
    }

    // The keyboard equivalent of the double click. A temporary tab is the one thing in this
    // strip whose state can be changed without opening or closing anything, and somebody
    // working without a pointer needs a way to do it.
    if (event.shiftKey && event.key === 'Enter') {
      event.preventDefault();
      workspace.pin(id);
    }
  }

  function onDrop(event: DragEvent, index: number): void {
    event.preventDefault();
    if (dragging !== null) {
      workspace.move(dragging, index);
    }
    dragging = null;
  }
</script>

<nav class="tab-bar" aria-label="Pestañas abiertas">
  <ul>
    {#each tabs as tab, index (tab.id)}
      {@const section = sectionOf(tab.section)}
      {@const Icon = section.icon}
      {@const active = tab.id === activeId}
      <li
        class="tab"
        class:active
        class:temporary={tab.temporary}
        style="--tab-colour: {section.colour}; --tab-tint: {section.tint}"
        draggable={tab.section !== 'panel'}
        ondragstart={() => (dragging = tab.id)}
        ondragend={() => (dragging = null)}
        ondragover={(event) => event.preventDefault()}
        ondrop={(event) => onDrop(event, index)}
      >
        <button
          type="button"
          class="title"
          data-tab={tab.id}
          aria-current={active ? 'page' : undefined}
          title={tab.temporary
            ? `${section.title} · doble clic o Mayús Intro para fijarla`
            : section.title}
          onclick={() => workspace.activate(tab.id)}
          ondblclick={() => workspace.pin(tab.id)}
          onkeydown={(event) => onTitleKeydown(event, tab.id, index)}
        >
          <Icon />
          <span class="text">{section.title}</span>
        </button>

        {#if section.closable}
          <button
            type="button"
            class="close"
            title="Cerrar {section.title}"
            onclick={() => workspace.close(tab.id)}
          >
            <IconClose label="Cerrar {section.title}" />
          </button>
        {/if}
      </li>
    {/each}
  </ul>
</nav>

<style>
  /* Part of the same piece as the header: one hairline between them, which the header
   * already draws, and this one closes the band against the content below it. */
  .tab-bar {
    flex: none;
    padding: 0 var(--space-3);
    background-color: var(--colour-surface-raised);
    border-bottom: var(--border-width) solid var(--colour-border-strong);
  }

  ul {
    display: flex;
    align-items: stretch;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
    overflow: hidden;
  }

  .tab {
    display: flex;
    align-items: center;
    min-width: 0;
    max-width: var(--tab-max-width);
    /* The top edge of the active tab lives here even when it is not drawn, so that
     * becoming active does not move the text by three pixels. */
    border-top: var(--rule-width) solid transparent;
    background-color: transparent;
  }

  .tab.active {
    border-top-color: var(--tab-colour);
    background-color: var(--tab-tint);
  }

  .title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: 0;
    background-color: transparent;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .tab.active .title {
    color: var(--colour-text);
    font-weight: var(--weight-medium);
  }

  /* A tab that is only being glanced at, and that the next thing opened will take over. */
  .tab.temporary .title {
    font-style: italic;
    text-decoration: underline dotted;
    text-underline-offset: var(--space-1);
  }

  /* A title is a strip of window, and in 02.1b it will be a name somebody chose. It is
   * truncated here and trimmed at the source; it is never hidden, because an icon alone on
   * a control whose meaning is not obvious is a control nobody can read. */
  .text {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .close {
    display: grid;
    flex: none;
    place-items: center;
    width: var(--control-min-size);
    height: var(--control-min-size);
    margin-right: var(--space-2);
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: var(--colour-text-faint);
    transition: background-color var(--duration-fast) var(--easing);
  }

  .close:hover {
    background-color: var(--colour-surface-sunken);
    color: var(--colour-text);
  }
</style>
