<script lang="ts">
  /**
   * The command palette: `Ctrl` `K`, type, run.
   *
   * Two lists under one field. Above, the commands — everything that can be opened and the
   * handful of actions that have no other home. Below, the global search, which in this
   * phase answers with nothing and says why.
   *
   * **No result here will ever show a value.** A search hit carries a module, an identity, a
   * title and a date, and there is no field for anything else; the type in
   * `src/lib/search/contract.ts` is what guarantees that rather than the care of whoever
   * writes this component. It matters here more than anywhere because this is the window
   * that opens on two keys, over three modules at once, in front of whoever is standing
   * behind the person who left the vault unlocked.
   *
   * The history is in memory and goes when the vault closes, like everything else in the
   * workspace. What somebody looked for is as good a record of their afternoon as what they
   * opened.
   *
   * The markup is the ARIA combobox pattern: the focus stays in the field and the active row
   * is named by `aria-activedescendant`, rather than moving the focus into the list. It is
   * the one place in this application with ARIA doing real work, and it is here because
   * there is no native element that is a text field and a list of results at once.
   */
  import { onMount } from 'svelte';

  import Badge from '../Badge.svelte';
  import IconSearch from '../../icons/IconSearch.svelte';
  import { MAX_HITS, MAX_QUERY, searchAll, type SearchHit } from '../../search/contract';
  import { PROVIDERS, SEARCH_IS_READY, SEARCH_NOTICE } from '../../search/providers';
  import { sectionOf } from '../sections';
  import { isClosable as closable } from '../tabs';
  import { COMMANDS } from './catalogue';
  import { rank, type Command } from './commands';
  import { workspace } from '../workspace.svelte';

  interface Props {
    /** Runs a command, and closes the palette. */
    onrun: (command: Command) => void;
    /** Closes it without running anything. */
    onclose: () => void;
  }

  const { onrun, onclose }: Props = $props();

  let query = $state('');
  let here = $state(0);
  let hits = $state<readonly SearchHit[]>([]);
  let field = $state<HTMLInputElement | null>(null);
  let list = $state<HTMLUListElement | null>(null);
  let dismiss = $state<HTMLButtonElement | null>(null);

  const history = $derived(workspace.state?.history ?? []);

  /**
   * Whether a command can do anything at this moment.
   *
   * Two of them cannot always. Reopening needs something to have been closed, and closing a
   * tab needs a tab that is allowed to close, which the panel is not.
   *
   * They are left out of the list rather than shown greyed. A menu is a fixed set of things
   * and an entry missing from one is confusing, which is why the menu disables its entry and
   * says why; a list of search results is expected to answer the query and nothing else, and
   * a row that does nothing when it is chosen is how an application starts looking broken.
   */
  function usable(command: Command): boolean {
    const tabs = workspace.state?.tabs;

    switch (command.effect.kind) {
      case 'reopenTab':
        return (tabs?.closed.length ?? 0) > 0;
      case 'closeTab':
        return (
          tabs !== undefined && tabs.tabs.some((tab) => tab.id === tabs.activeId && closable(tab))
        );
      default:
        return true;
    }
  }

  const found = $derived(rank(COMMANDS.filter(usable), query, history));
  const active = $derived(found[Math.min(here, Math.max(found.length - 1, 0))]);

  onMount(() => {
    field?.focus();
  });

  // The search runs against the real providers even while all three answer with nothing.
  // Wiring it up now is what proves the contract is usable; stubbing the call out would
  // leave the first real test of it for the phase that can least afford a surprise.
  $effect(() => {
    const wanted = query;
    let current = true;

    void searchAll(PROVIDERS, wanted, MAX_HITS).then((answer) => {
      // A slower answer to an older query must not overwrite a newer one.
      if (current) {
        hits = answer;
      }
    });

    return () => {
      current = false;
    };
  });

  // The focus never leaves the field, so the browser will not scroll the chosen row into
  // view on its own the way it would if the row had been tabbed to. That is the price of
  // `aria-activedescendant`, and this is the part of the price that has to be paid in code.
  $effect(() => {
    const id = active?.id;
    if (id === undefined) {
      return;
    }
    document
      .getElementById(`palette-${id}`)
      ?.scrollIntoView({ block: 'nearest', behavior: 'instant' });
  });

  function move(step: number): void {
    if (found.length === 0) {
      return;
    }
    here = (((here + step) % found.length) + found.length) % found.length;
  }

  function run(command: Command): void {
    workspace.remember(command.id);
    onrun(command);
  }

  function onKeydown(event: KeyboardEvent): void {
    switch (event.key) {
      case 'Escape':
        event.preventDefault();
        onclose();
        break;
      case 'ArrowDown':
        event.preventDefault();
        move(1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        move(-1);
        break;
      case 'Home':
        event.preventDefault();
        here = 0;
        break;
      case 'End':
        event.preventDefault();
        here = Math.max(found.length - 1, 0);
        break;
      case 'Enter':
        event.preventDefault();
        if (active !== undefined) {
          run(active);
        }
        break;
      case 'Tab': {
        // The focus stays inside: this is a layer over the whole application, and `Tab`
        // walking off into the screen behind it would leave somebody typing into a field
        // they can no longer see.
        //
        // Three stops rather than two, because the list of results is a scrollable region
        // and a scrollable region a pointer can reach and a keyboard cannot is a barrier.
        // The arrows move the chosen row and carry the scroll with them from wherever the
        // focus is, so the stop is a way in rather than the only way.
        event.preventDefault();
        const stops = [field, list, dismiss].filter((stop) => stop !== null);
        const at = stops.findIndex((stop) => stop === document.activeElement);
        const step = event.shiftKey ? -1 : 1;
        stops[(((at + step) % stops.length) + stops.length) % stops.length]?.focus();
        break;
      }
      default:
        break;
    }
  }
</script>

<!--
  The backdrop closes the palette when pressed, which is what a layer does. It is not the
  only way out: `Escape` closes it, and so does the button in the corner, so no keyboard
  equivalent is missing here.
-->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={onclose}>
  <div
    class="palette"
    role="dialog"
    aria-modal="true"
    aria-label="Paleta de comandos"
    tabindex="-1"
    onclick={(event) => event.stopPropagation()}
    onkeydown={onKeydown}
  >
    <div class="field">
      <label class="label" for="palette-query">Buscar o ejecutar</label>

      <div class="input">
        <IconSearch />
        <input
          bind:this={field}
          bind:value={query}
          oninput={() => (here = 0)}
          id="palette-query"
          type="text"
          maxlength={MAX_QUERY}
          role="combobox"
          autocomplete="off"
          spellcheck="false"
          aria-expanded="true"
          aria-controls="palette-commands"
          aria-activedescendant={active === undefined ? undefined : `palette-${active.id}`}
        />
        <button bind:this={dismiss} type="button" class="dismiss" onclick={onclose}>Cerrar</button>
      </div>
    </div>

    <div class="results">
      <ul bind:this={list} id="palette-commands" role="listbox" aria-label="Comandos" tabindex="0">
        {#each found as command (command.id)}
          {@const chosen = command.id === active?.id}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <li
            id="palette-{command.id}"
            role="option"
            aria-selected={chosen}
            class="row"
            class:chosen
            onclick={() => run(command)}
          >
            <span class="text">
              <span class="title">{command.title}</span>
              <span class="detail">{command.group} · {command.detail}</span>
            </span>
            {#if command.shortcut !== null}
              <span class="shortcut">{command.shortcut}</span>
            {/if}
          </li>
        {/each}

        {#if found.length === 0}
          <li class="none">Ningún comando coincide con «{query}».</li>
        {/if}
      </ul>

      <section class="search">
        <div class="search-heading">
          <span class="label">Buscar en tus datos</span>
          {#if !SEARCH_IS_READY}
            <Badge tone={sectionOf('habits').tone} text="En desarrollo" />
          {/if}
        </div>

        {#if !SEARCH_IS_READY}
          <p class="notice">{SEARCH_NOTICE}</p>
        {:else if hits.length === 0}
          <p class="notice">Nada coincide con «{query}».</p>
        {:else}
          <ul class="hits">
            {#each hits as hit (`${hit.module}:${hit.id}`)}
              {@const section = sectionOf(hit.module)}
              <li class={section.tone}>
                <span class="chip mark" aria-hidden="true"></span>
                <span class="title">{hit.title}</span>
                <span class="detail">{section.title}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    z-index: 3;
    inset: 0;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: var(--space-8) var(--space-5);
    /* No dimming layer: a tint over the whole window would be a colour that is not a token
     * and a second thing competing with the palette for the eye. The layer is told apart
     * from what is under it by its border and its raised surface. */
    background-color: transparent;
  }

  .palette {
    display: flex;
    flex-direction: column;
    width: var(--layer-width-palette);
    max-width: 100%;
    max-height: 100%;
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    border-bottom: var(--border-width) solid var(--colour-border);
  }

  .input {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: var(--colour-surface-sunken);
    color: var(--colour-text-muted);
  }

  .input input {
    flex: 1;
    min-width: 0;
    border: 0;
    background-color: transparent;
    color: var(--colour-text);
    font-size: var(--text-lg);
  }

  .input input:focus {
    outline: none;
  }

  /* The ring goes on the box rather than on the field inside it, because the box is what
   * reads as the control. */
  .input:focus-within {
    outline: var(--rule-width) solid var(--colour-accent);
    outline-offset: var(--border-width);
  }

  .dismiss {
    flex: none;
    padding: var(--space-1) var(--space-2);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: var(--colour-text-muted);
    font-size: var(--text-xs);
  }

  .dismiss:hover {
    background-color: var(--colour-surface);
    color: var(--colour-text);
  }

  .results {
    display: flex;
    min-height: 0;
    flex-direction: column;
    padding: var(--space-2);
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* The list is what scrolls, rather than the whole layer, so the field stays put while it
   * is being typed into and the search section stays where the eye left it. It is as tall
   * as it needs to be and no taller: the layer only reaches the bottom of the window when
   * there is more than a window's worth of commands. */
  #palette-commands {
    min-height: 0;
    overflow: auto;
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-left: var(--rule-width) solid transparent;
    border-radius: var(--radius-sm);
  }

  .row:hover {
    background-color: var(--colour-surface-sunken);
  }

  /* The chosen row is the one Enter runs. It is a border and a surface rather than a tint,
   * so it survives forced colours and reads without relying on hue alone. */
  .chosen {
    border-left-color: var(--colour-accent);
    background-color: var(--colour-surface-sunken);
  }

  .text {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: var(--space-1);
  }

  .title {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .detail {
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .shortcut {
    margin-left: auto;
    flex: none;
    color: var(--colour-text-faint);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .none,
  .notice {
    margin: 0;
    padding: var(--space-3);
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  .search {
    margin-top: var(--space-3);
    padding-top: var(--space-3);
    border-top: var(--border-width) solid var(--colour-border);
  }

  .search-heading {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-3);
  }

  .hits li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
  }

  .chip {
    width: var(--chip-size);
    height: var(--chip-size);
    flex: none;
    background-color: var(--tone-mark);
  }
</style>
