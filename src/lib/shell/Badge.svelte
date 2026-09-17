<script lang="ts">
  /**
   * The small uppercase badge that says a part is not written yet.
   *
   * It goes where the value will go, in the colour of the module it belongs to. The design
   * system asks for it on anything drawn before it works, and it is a component rather than
   * a class so that every one of them reads the same and so that the day the badges come
   * out, they come out of one place.
   */

  interface Props {
    /** The module's colour, as a token reference. */
    colour: string;
    /** The module's tint, which the badge sits on. */
    tint: string;
    /** What it says. Short: it is read at a glance, not studied. */
    text: string;
  }

  const { colour, tint, text }: Props = $props();
</script>

<span class="badge" style="--badge-colour: {colour}; --badge-tint: {tint}">{text}</span>

<style>
  .badge {
    display: inline-block;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background-color: var(--badge-tint);
    color: var(--badge-colour);
    font-size: var(--text-label);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
  }
</style>
