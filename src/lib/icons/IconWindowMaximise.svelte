<script lang="ts">
  /**
   * The middle control of the undecorated window: maximise, and restore.
   *
   * One icon with two states rather than two icons, because it is one button. The glyph
   * has to say what pressing it will do, and a button whose icon never changed would be
   * lying half the time.
   *
   * The box, the stroke and the decision about assistive technology all live in
   * `IconFrame.svelte`. What is here is the geometry.
   */
  import IconFrame from './IconFrame.svelte';

  interface Props {
    /** What to call this icon out loud, when it is the only thing identifying a control. */
    label?: string | undefined;
    /** Whether the window is maximised, and so whether pressing this will restore it. */
    maximised: boolean;
  }

  const { label, maximised }: Props = $props();
</script>

<IconFrame {label}>
  {#if maximised}
    <!-- Two overlapping squares: the window is maximised and this puts it back. -->
    <path d="M8.25 8.25V6.5a1 1 0 0 1 1-1h8.25a1 1 0 0 1 1 1v8.25a1 1 0 0 1-1 1h-1.75" />
    <rect x="5.5" y="8.25" width="10.25" height="10.25" rx="1" />
  {:else}
    <!-- One square: the window is not maximised and this fills the screen with it. -->
    <rect x="5.5" y="5.5" width="13" height="13" rx="1" />
  {/if}
</IconFrame>
