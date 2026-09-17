<script lang="ts">
  /**
   * Diagnostics: a trace. It is the screen somebody opens when they want to know what the machine is actually doing.
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
  <path d="M2.5 12h4l2.5-6.5 4 13 2.5-6.5h6" />
</IconFrame>
