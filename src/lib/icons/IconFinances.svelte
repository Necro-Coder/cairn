<script lang="ts">
  /**
   * Finances: columns on an axis. Money in this application is read as totals over time.
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
  <path d="M3 20.5h18" />
  <path d="M6.75 20.5v-5.75M12 20.5v-11M17.25 20.5v-7.75" />
</IconFrame>
