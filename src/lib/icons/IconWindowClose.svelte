<script lang="ts">
  /**
   * The window control, drawn at full size so that it cannot be mistaken for the small cross that closes a tab.
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
  <path d="M5.75 5.75l12.5 12.5M18.25 5.75L5.75 18.25" />
</IconFrame>
