<script lang="ts">
  /**
   * The search field in the palette, and nothing else. A magnifier anywhere else would promise a search that is not there.
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
  <circle cx="11" cy="11" r="6.5" />
  <path d="M15.75 15.75L20.5 20.5" />
</IconFrame>
