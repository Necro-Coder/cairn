<script lang="ts">
  /**
   * Habits: a calendar with a day ticked, which is the whole of what a habit is.
   *
   * The box, the stroke and the decision about assistive technology all live in
   * `IconFrame.svelte`. What is here is the geometry.
   */
  import IconFrame from './IconFrame.svelte';

  interface Props {
    /** What to call this icon out loud, when it is the only thing identifying a control. */
    label?: string | undefined;
  }

  const { label }: Props = $props();
</script>

<IconFrame {label}>
  <rect x="3.5" y="5" width="17" height="15.5" />
  <path d="M8 2.75v3.5M16 2.75v3.5M3.5 10h17" />
  <path d="M9.25 14.75l2.25 2.25 4-4.25" />
</IconFrame>
