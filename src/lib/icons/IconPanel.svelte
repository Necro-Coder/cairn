<script lang="ts">
  /**
   * The panel: four cells of a grid, because the panel is what the person arranges cards in.
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
  <rect x="3.5" y="3.5" width="7" height="7" />
  <rect x="13.5" y="3.5" width="7" height="7" />
  <rect x="3.5" y="13.5" width="7" height="7" />
  <rect x="13.5" y="13.5" width="7" height="7" />
</IconFrame>
