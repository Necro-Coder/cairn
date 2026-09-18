<script lang="ts">
  /**
   * A quiet line on the panel when it has been a while since the last backup.
   *
   * Not a modal and not a nag. It appears after fourteen days without a copy, or from the
   * first day if there has never been one, and it is a sentence with a link in it. The person
   * decides when to make a backup; the only useful thing this can do is make sure they are
   * deciding rather than forgetting.
   *
   * It asks the core rather than remembering anything itself, and what the core answers is a
   * number of days and a yes or no. The folder the last copy went to never crosses the
   * boundary: a path carries the account name and very often the machine name.
   *
   * A failure to ask is a failure to show anything. A vault that is closed has no history to
   * read, and a panel that announced an error about a reminder would be worse than a panel
   * with no reminder on it.
   */
  import { ipc } from '$ipc';
  import { onMount } from 'svelte';

  interface Props {
    /** What to do when somebody follows the link, which is to open the data settings. */
    readonly onopen: () => void;
  }

  const { onopen }: Props = $props();

  let daysSinceLast = $state<number | null>(null);
  let remind = $state(false);

  onMount(() => {
    let alive = true;

    void ipc
      .backupStatus()
      .then((status) => {
        if (alive) {
          daysSinceLast = status.daysSinceLast;
          remind = status.remind;
        }
      })
      .catch(() => {
        // Nothing to say. See the note at the top.
      });

    return () => {
      alive = false;
    };
  });
</script>

{#if remind}
  <p class="reminder" role="note">
    {#if daysSinceLast === null}
      Todavía no has hecho ninguna copia de seguridad.
    {:else}
      Hace {daysSinceLast} días de tu última copia de seguridad.
    {/if}
    <button type="button" onclick={onopen}>Hacer una copia</button>
  </p>
{/if}

<style>
  /*
   * A line, not a banner. One border in the warning colour and nothing else: no background
   * tint, no icon, no dismissal. Something that can be dismissed is something that gets
   * dismissed once and never seen again, and this is the sentence that matters on the day
   * somebody's disk fails.
   */
  .reminder {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-3);
    max-width: var(--measure);
    margin: 0 0 var(--space-5) 0;
    padding: var(--space-3) var(--space-4);
    border-left: var(--rule-width) solid var(--colour-warning);
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  /* Plain text underlined on hover, which is what the design system calls a third action. */
  .reminder button {
    padding: 0;
    border: none;
    background: none;
    color: var(--colour-text);
    font: inherit;
    text-decoration: underline;
  }
</style>
