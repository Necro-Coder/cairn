<script lang="ts">
  /**
   * The habits section: the list of what today asks for, or one habit opened.
   *
   * The two screens swap in place rather than becoming two tabs. A habit is not a place in
   * the application, it is something being looked at inside habits, and the tab strip is for
   * the five sections. Going back is a control on the screen and it always returns to the
   * list, which is the only route in.
   *
   * What was here before was the shape of the module drawn with nothing behind it. It did its
   * job — it is what the design was settled against — and there is a core behind it now.
   */
  import HabitDetailScreen from '../habits/HabitDetailScreen.svelte';
  import TodayScreen from '../habits/TodayScreen.svelte';

  /** Which habit is open, or nothing, which is the list. */
  let open = $state<string | null>(null);
</script>

{#if open === null}
  <TodayScreen onOpen={(id) => (open = id)} />
{:else}
  <HabitDetailScreen id={open} onBack={() => (open = null)} />
{/if}
