<script lang="ts">
  /**
   * Settings: the gear everybody already recognises. Inventing a new one here would cost more than it gained.
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
  <circle cx="12" cy="12" r="3.25" />
  <path d="M12 3.25v3.25M12 17.5v3.25M3.25 12h3.25M17.5 12h3.25" />
  <path d="M5.85 5.85l2.3 2.3M15.85 15.85l2.3 2.3M18.15 5.85l-2.3 2.3M8.15 15.85l-2.3 2.3" />
</IconFrame>
