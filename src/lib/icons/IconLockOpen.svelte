<script lang="ts">
  /**
   * The vault is open. The shackle is lifted on one side only, so the difference is visible at sixteen pixels.
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
  <rect x="3.75" y="10.5" width="16.5" height="10" rx="1" />
  <path d="M7.75 10.5V7a4.25 4.25 0 0 1 8.5 0" />
</IconFrame>
