<script lang="ts">
  /**
   * The habits section: the list of what today asks for, one habit opened, or the form.
   *
   * The three swap in place rather than becoming three tabs. A habit is not a place in the
   * application, it is something being looked at inside habits, and the strip is for the five
   * sections. Every way out of the other two leads back to the list, which is the only route
   * in, so there is no state here that anybody can get lost in.
   *
   * What was here before was the shape of the module drawn with nothing behind it. It did its
   * job — it is what the design was settled against — and there is a core behind it now.
   */
  import HabitDetailScreen from '../habits/HabitDetailScreen.svelte';
  import HabitFormScreen from '../habits/HabitFormScreen.svelte';
  import TodayScreen from '../habits/TodayScreen.svelte';

  /** Which of the three is on screen, and what it is about. */
  type Where =
    | { readonly at: 'list' }
    | { readonly at: 'habit'; readonly id: string }
    | { readonly at: 'form'; readonly id: string | null };

  let where = $state<Where>({ at: 'list' });
</script>

{#if where.at === 'list'}
  <TodayScreen
    onOpen={(id) => (where = { at: 'habit', id })}
    onCreate={() => (where = { at: 'form', id: null })}
  />
{:else if where.at === 'habit'}
  {@const opened = where.id}
  <HabitDetailScreen
    id={opened}
    onBack={() => (where = { at: 'list' })}
    onEdit={() => (where = { at: 'form', id: opened })}
  />
{:else}
  <HabitFormScreen
    id={where.id}
    onDone={(savedId) => (where = savedId === null ? { at: 'list' } : { at: 'habit', id: savedId })}
  />
{/if}
