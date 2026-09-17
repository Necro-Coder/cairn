<script lang="ts">
  /**
   * A tab that has been made permanent. A pin rather than a padlock: nothing here is locked, it is held in place.
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
  <path d="M12 15.5v5.25" />
  <path d="M8.5 3.25h7l-1 6.25 3.25 3.5H6.25l3.25-3.5z" />
</IconFrame>
