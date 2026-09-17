<script lang="ts">
  /**
   * Close a tab, remove a card, dismiss a layer. Drawn smaller than the window control it would otherwise be confused with.
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
  <path d="M7.5 7.5l9 9M16.5 7.5l-9 9" />
</IconFrame>
