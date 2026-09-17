<script lang="ts">
  /**
   * Passwords: a key. The one icon in the set whose meaning nobody has to be taught.
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
  <circle cx="8.5" cy="12" r="4.25" />
  <path d="M12.75 12h8.25M18.5 12v2.75M15.75 12v2" />
</IconFrame>
