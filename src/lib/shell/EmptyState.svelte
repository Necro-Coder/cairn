<script lang="ts">
  /**
   * What a list looks like before anybody has put anything in it.
   *
   * The same three pieces every time, because the design system asks for exactly those
   * three: a small geometric composition, one sentence saying what will be here, and the
   * action that puts something there. Never the word "empty" on its own.
   *
   * It is a component rather than a pattern copied three times for the reason the rule of
   * three exists: habits, passwords and finances all need it on the same day, and the day
   * one of the three modules starts working, only one of these has to stop saying so.
   *
   * The badge is permanent and the sentence under the button is not. That is the answer to
   * two rules that arrive together: the part has to say it is unwritten wherever the value
   * will go, and the button has to still be pressable and explain itself when it is. A
   * control that does nothing looks like a broken application; one that explains looks like
   * an unfinished one, which is what this is.
   */
  import Badge from './Badge.svelte';
  import Marks from './Marks.svelte';
  import type { Section } from './sections';

  interface Props {
    /** Which module this list belongs to, which is where the badge's colour comes from. */
    section: Section;
    /** One sentence, in the person's own terms, saying what will be here. */
    sentence: string;
    /** What the action that puts something here is called. */
    action: string;
    /** What pressing it says, when there is nothing behind it yet. A real action needs none. */
    note?: string;
    /**
     * What pressing it actually does, once there is something behind it.
     *
     * Absent is the state this component was written for: a button drawn before the part it
     * belongs to works, which still reacts and explains itself, because a control that does
     * nothing looks like a broken application and one that explains looks like an unfinished
     * one. Present is the other state, and the badge comes off with it — a part that works
     * has nothing to say about being in development.
     */
    onAction?: (() => void) | undefined;
  }

  const { section, sentence, action, note = '', onAction }: Props = $props();

  let explained = $state(false);
</script>

<div class="empty">
  <Marks />

  <p class="sentence">{sentence}</p>

  <div class="actions">
    <button
      type="button"
      class="primary"
      onclick={() => {
        if (onAction === undefined) {
          explained = true;
        } else {
          onAction();
        }
      }}>{action}</button
    >
    {#if onAction === undefined}
      <Badge tone={section.tone} text="En desarrollo" />
    {/if}
  </div>

  {#if explained}
    <p class="note">{note}</p>
  {/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-5);
    padding: var(--space-7) 0;
  }

  .sentence {
    max-width: var(--measure);
    margin: 0;
    font-size: var(--text-lg);
    color: var(--colour-text-muted);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
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

  /* Beside what caused it, not floating over the window: an error or a notice that takes
   * itself away is one nobody finished reading. */
  .note {
    max-width: var(--measure);
    margin: 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }
</style>
