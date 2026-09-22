<script lang="ts" generics="T">
  /**
   * Draws whichever of the four states a screen is in, so that no screen draws two of them.
   *
   * It owns `loading` and `failed` outright, because those two look the same everywhere and
   * writing them once is the difference between one waiting indicator and nine of them that
   * drifted. `empty` and `ready` are snippets, because those two are the screen itself: the
   * first day of a list of habits is not the first day of a year of squares.
   *
   * The `{#if}` chain is exhaustive over the union and ends in nothing, which is reachable
   * only if somebody adds a fifth state. That is caught by `messageFor` in `async.ts` first,
   * where the compiler enforces it; here it merely means an unfinished screen draws nothing
   * rather than the wrong thing.
   *
   * The waiting indicator is never drawn before `--delay-loading`. It is done with an
   * animation delay rather than a timer in this component on purpose: a timer is state to
   * start, clear and get wrong on unmount, and the browser already has a clock. A person who
   * asked for reduced motion still gets the delay — what they lose is the fade, which is
   * exactly the right half to lose.
   */
  import type { Snippet } from 'svelte';

  import { messageFor, type Async } from './async';

  interface Props {
    /** What the screen is doing, as one value. */
    state: Async<T>;
    /** What to draw when the answer arrived and there is nothing in it. */
    empty: Snippet;
    /** What to draw when there is something. */
    ready: Snippet<[T]>;
    /** What is being waited for, for the people who cannot see the indicator. */
    label: string;
  }

  const { state, empty, ready, label }: Props = $props();
</script>

{#if state.status === 'loading'}
  <!-- No live region. The design system says so once, for the whole application, and the
       reason is written there: the AA floor done well rather than half of AAA badly. -->
  <p class="waiting">{label}</p>
{:else if state.status === 'empty'}
  {@render empty()}
{:else if state.status === 'ready'}
  {@render ready(state.value)}
{:else if state.status === 'failed'}
  <!-- Inside the screen, where it happened. Never a notice that takes itself away. -->
  <p class="failure">{messageFor(state.error)}</p>
{/if}

<style>
  /*
   * Absent until the wait is long enough to be worth admitting to.
   *
   * `backwards` is what makes the first frame count: without it the element would be drawn
   * at full opacity for the whole delay and the delay would buy nothing.
   */
  .waiting {
    margin: 0;
    padding: var(--space-5) 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
    opacity: 0;
    animation: appear var(--duration-normal) var(--easing) var(--delay-loading) forwards backwards;
  }

  @keyframes appear {
    from {
      opacity: 0;
    }

    to {
      opacity: 1;
    }
  }

  /* The warning colour, a border beside it, and the whole sentence in plain Spanish. */
  .failure {
    max-width: var(--measure);
    margin: 0;
    padding: var(--space-4) var(--space-5);
    border-left: var(--rule-width) solid var(--colour-negative);
    background-color: var(--colour-surface-sunken);
    color: var(--colour-text);
  }
</style>
