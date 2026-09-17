<script lang="ts">
  /**
   * The title block of a screen: the section it belongs to, the title, and the rule.
   *
   * The same on every screen, which is the point. The label above the title in the tracked
   * uppercase style and the thick rule under it are the two pieces of typographic
   * furniture this system is built on, and a screen that drew its own version of them
   * would be a screen that looked like it came from somewhere else.
   *
   * The geometry, where a screen is allowed any, sits to the right in a region of its own
   * with nothing on top of it.
   */
  import Marks from './Marks.svelte';
  import type { Section } from './sections';

  interface Props {
    /** Which section this screen belongs to, which is where its colour comes from. */
    section: Section;
    /** The title, which is usually but not always the section's own name. */
    title: string;
    /** One sentence under the rule, where the screen needs one. */
    lede?: string | undefined;
    /**
     * Whether this screen is one of the ones allowed a composition.
     *
     * Not a matter of taste: the list is closed and is in the design system. A screen with
     * data on it never passes true.
     */
    marks?: boolean;
  }

  const { section, title, lede, marks = false }: Props = $props();
</script>

<div class="screen-header" style="--screen-mark: {section.mark}">
  <div class="block">
    <span class="named">
      <span class="chip mark" aria-hidden="true"></span>
      <span class="label">{section.title}</span>
    </span>

    <h1>{title}</h1>
    <div class="rule" aria-hidden="true"></div>

    {#if lede !== undefined}
      <p class="lede">{lede}</p>
    {/if}
  </div>

  {#if marks}
    <Marks />
  {/if}
</div>

<style>
  .screen-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-6);
  }

  .block {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
  }

  .named {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .chip {
    width: var(--chip-size);
    height: var(--chip-size);
    background-color: var(--screen-mark);
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

  .lede {
    margin-top: var(--space-4);
    color: var(--colour-text-muted);
  }
</style>
