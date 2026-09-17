<script lang="ts">
  /**
   * Another copy of the application already has this data directory, the lock could not be taken
   * at all, or there was no directory to take it in.
   *
   * The whole screen is one message and one button, because there is exactly one thing to do
   * and the person did not ask for any of this. What it must not do is offer to carry on
   * anyway: two processes with the same database open is two logical clocks handing out the
   * same readings, and that produces rows no later merge can order.
   *
   * It says which of the three problems it is, because they lead to different places. Somebody
   * told "another copy has it" goes looking for a window. Somebody told that would not find the
   * disk that is full, and neither of them would find the profile name they mistyped, which is
   * why each case says something of its own.
   *
   * Nothing on this screen names a path, a process or an account. It is drawn before the
   * vault has been read, so there is nothing to name, and that is by construction rather
   * than by care.
   */
  import Marks from '../lib/shell/Marks.svelte';
  import type { InstanceState } from '../lib/ipc.types';

  interface Props {
    /** Which of the three refusals happened. */
    state: InstanceState;
    /** Closes the window. The only action this screen offers. */
    onclose: () => void;
  }

  const { state, onclose }: Props = $props();
</script>

<section class="refused">
  <div class="column">
    <span class="label">Cairn</span>
    {#if state === 'alreadyRunning'}
      <h1>Ya hay una copia abierta</h1>
      <div class="rule" aria-hidden="true"></div>

      <p>
        Cairn ya se está ejecutando en este equipo y solo puede haber una copia usando los mismos
        datos. Busca la ventana que ya está abierta: está en la barra de tareas.
      </p>

      <p class="what-to-do" role="note">
        No se ha tocado nada. Esta copia no ha llegado a abrir la caja fuerte ni a escribir en
        ningún fichero.
      </p>
    {:else if state === 'noDirectory'}
      <h1>No hay dónde guardar la caja</h1>
      <div class="rule" aria-hidden="true"></div>

      <p>
        Cairn no ha podido decidir en qué carpeta vive el vault, así que no ha abierto ninguna. Si
        has arrancado con la variable <code>CAIRN_PROFILE</code>, el nombre que lleva no vale: solo
        admite letras, números, guiones y guiones bajos, hasta treinta y dos caracteres, y nunca una
        ruta. Quítala o corrígela y vuelve a abrir.
      </p>

      <p class="what-to-do" role="note">
        No se ha tocado nada. Esta copia no ha llegado a abrir la caja fuerte ni a escribir en
        ningún fichero, y tu vault de siempre sigue donde estaba.
      </p>
    {:else}
      <h1>No se ha podido reservar la carpeta de datos</h1>
      <div class="rule" aria-hidden="true"></div>

      <p>
        Cairn no ha podido crear su fichero de bloqueo. No es otra copia abierta: es algo del
        equipo. Lo habitual es que el disco esté lleno, que la carpeta de datos ya no exista o que
        se hayan cambiado los permisos.
      </p>

      <p class="what-to-do" role="note">
        No se ha tocado nada. Esta copia no ha llegado a abrir la caja fuerte ni a escribir en
        ningún fichero.
      </p>
    {/if}

    <button type="button" onclick={onclose}>Cerrar esta ventana</button>
  </div>

  <Marks shapes={2} />
</section>

<style>
  .refused {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-6);
    width: 100%;
    max-width: var(--content-max);
    margin: 0 auto;
  }

  .column {
    display: flex;
    max-width: var(--measure);
    flex-direction: column;
    align-items: flex-start;
  }

  h1 {
    margin-top: var(--space-3);
  }

  .rule {
    width: var(--space-8);
    height: var(--rule-width);
    margin-top: var(--space-4);
    background-color: var(--colour-accent);
  }

  p {
    margin-top: var(--space-4);
  }

  .what-to-do {
    padding: var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
  }

  button {
    margin-top: var(--space-6);
    padding: var(--space-3) var(--space-5);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font-size: var(--text-sm);
  }
</style>
