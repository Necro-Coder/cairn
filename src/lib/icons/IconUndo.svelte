<script lang="ts">
  /**
   * Undo a deletion while the few seconds last. Nothing in this application is deleted for real, so undo is cheap and honest.
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
  <path d="M3.5 9.5h10a5.5 5.5 0 0 1 0 11H8" />
  <path d="M7.5 5.5l-4 4 4 4" />
</IconFrame>
