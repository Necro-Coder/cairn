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
    /**
     * The class that carries the module's colours, from its entry in `sections.ts`.
     *
     * A class rather than the two values, because the content security policy forbids
     * inline styles and a `style` attribute is the only other way to pass them in.
     */
    tone: string;
    /** What it says. Short: it is read at a glance, not studied. */
    text: string;
  }

  const { tone, text }: Props = $props();
</script>

<span class="badge {tone}">{text}</span>

<style>
  .badge {
    display: inline-block;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background-color: var(--tone-tint);
    color: var(--tone-colour);
    font-size: var(--text-label);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
  }
</style>
