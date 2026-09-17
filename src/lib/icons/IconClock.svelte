<script lang="ts">
  /**
   * Session history, and reopening the last closed tab. Both are questions about what happened a moment ago.
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
  <circle cx="12" cy="12" r="8.25" />
  <path d="M12 7v5.25l3.25 2" />
</IconFrame>
