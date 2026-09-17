<script lang="ts">
  import { ipc } from '$ipc';

  import CreateVaultScreen from './routes/CreateVaultScreen.svelte';
  import DamagedScreen from './routes/DamagedScreen.svelte';
  import DiagnosticsScreen from './routes/DiagnosticsScreen.svelte';
  import SecondCopyScreen from './routes/SecondCopyScreen.svelte';
  import UnlockScreen from './routes/UnlockScreen.svelte';
  import Shell from './lib/shell/Shell.svelte';
  import TitleBar from './lib/shell/TitleBar.svelte';
  import { measureStartup, type StartupResult } from './lib/startup';
  import { session } from './lib/session.svelte';
  import { workspace } from './lib/shell/workspace.svelte';
  import type { InstanceState, VaultStatus } from './lib/ipc.types';

  // Null in every real build, so the banner below is not hidden by a condition: its text
  // does not exist in the bundle at all. The module that invents the data is the one that
  // says so, which is why this comes across the boundary rather than from a build flag.
  const previewNotice = ipc.previewNotice;

  let startup = $state<StartupResult | null>(null);
  let diagnosticsOpen = $state(false);
  let ready = $state(false);

  /**
   * Whether this copy owns the data directory, and what to say if it does not.
   *
   * Asked before anything else, because everything else assumes the vault is reachable and in
   * a second copy it is not: the core never read the header, so `session.refresh()` would be
   * asking a state that was deliberately never managed.
   *
   * It starts as `held` so that the first frame is the ordinary one. A refusal is rare and a
   * flash of the refusal screen on every start would be worse than a frame of the usual one.
   */
  let instance = $state<InstanceState>('held');

  void ipc
    .fetchInstanceStatus()
    .then((status) => {
      instance = status.state;
    })
    .catch(() => {
      // The command boundary is broken, which the screens below already report where somebody
      // is looking. Assuming the worst here would replace a legible failure with a wrong
      // explanation of it.
      instance = 'held';
    });

  /** Whether the vault is reachable at all, which is what decides between two whole screens. */
  const owned = $derived(instance === 'held' || instance === 'guaranteedByThePlatform');

  // Measured as soon as the interface exists, so that the cold start figure is the time
  // the person waited rather than however long they took to press something.
  void measureStartup()
    .then((result) => {
      startup = result;
    })
    .catch(() => {
      // A failure here means the command boundary is broken, and the screen below already
      // reports that in a place the person is looking at. Repeating it would add noise
      // without adding information, and there is nothing to log to.
      startup = null;
    });

  // Read before anything is drawn, because what to draw is entirely this answer: a machine
  // with no vault, a vault that is closed, or one that is open.
  void session
    .refresh()
    .catch(() => undefined)
    .finally(() => {
      ready = true;
    });

  $effect(() => {
    // Started here rather than at module load so that it stops when the interface goes away,
    // which is what keeps a timer from outliving the thing it was reporting for.
    let stop: (() => void) | null = null;
    void session.start().then((cancel) => {
      stop = cancel;
    });

    return () => {
      stop?.();
      session.stop();
    };
  });

  function adopt(status: VaultStatus): void {
    session.adopt(status);
  }

  /**
   * The shortcuts that work wherever the vault is, and the activity report.
   *
   * Everything that moves between sections lives in the shell instead, because a section
   * is not something to open when there is nothing to open it onto.
   */
  function handleKeydown(event: KeyboardEvent): void {
    session.noteActivity();

    if (!event.ctrlKey || event.altKey || event.metaKey) {
      if (event.key === 'Escape' && diagnosticsOpen) {
        event.preventDefault();
        diagnosticsOpen = false;
      }
      return;
    }

    // Control and Shift together, so that nothing typed by accident opens a screen nobody
    // asked for. The shortcut works in release builds too: the machine where something
    // goes wrong is rarely the one with a debugger attached.
    //
    // It leads to two different places on purpose. With the vault open the diagnostics are
    // the fourth part of the settings screen, which is where they belong and where nothing
    // has to remember to close them. With it closed they are a screen of their own, because
    // the machine somebody needs them on is usually the one that will not open.
    if (event.shiftKey && event.key.toLowerCase() === 'd') {
      event.preventDefault();
      if (session.status.unlocked) {
        workspace.open('settings', { temporary: true });
        workspace.openSettings('diagnostics');
      } else {
        diagnosticsOpen = !diagnosticsOpen;
      }
      return;
    }

    if (!event.shiftKey && event.key.toLowerCase() === 'l' && session.status.unlocked) {
      event.preventDefault();
      void session.lock();
    }
  }
</script>

<svelte:window
  onkeydown={handleKeydown}
  onpointerdown={() => session.noteActivity()}
  onpointermove={() => session.noteActivity()}
/>

{#if previewNotice !== null}
  <!--
    Sticky rather than fixed, so it takes its own row instead of covering the first one,
    and with no way to close it. A warning that can be dismissed is a warning that will be,
    two minutes into looking at a screen and an hour before somebody decides something
    based on numbers this build invented.
  -->
  <p class="preview-banner" role="alert">{previewNotice}</p>
{/if}

<!--
  The order of these matters. An open vault wins over everything, so the diagnostics screen
  below cannot survive an unlock the way the panel it replaces survived a lock: there is no
  state left over to strand, because the branch it lives in is unreachable while the vault is
  open. That is the patch from PR #31 retired rather than moved.
-->
{#if !owned}
  <!--
    Before everything, because a second copy never opened the vault and every branch below
    asks about one. The title bar comes with it: a window with no system decoration that
    could not be moved or closed would be a trap, and this is the branch a person most wants
    to close.
  -->
  <TitleBar tone="plain" />
  <main class="plain">
    <SecondCopyScreen state={instance} onclose={() => void ipc.closeWindow()} />
  </main>
{:else if !ready}
  <TitleBar />
  <main class="plain">
    <p class="muted">Leyendo el estado de la caja fuerte…</p>
  </main>
{:else if session.status.unlocked}
  <Shell {startup} />
{:else if diagnosticsOpen}
  <TitleBar tone="plain" />
  <main class="plain">
    <DiagnosticsScreen {startup} onclose={() => (diagnosticsOpen = false)} />
  </main>
{:else if session.status.condition === 'unreadable'}
  <TitleBar tone="plain" />
  <main class="plain">
    <DamagedScreen />
  </main>
{:else if !session.status.exists}
  <TitleBar tone="plain" />
  <main class="plain">
    <CreateVaultScreen oncreated={adopt} />
  </main>
{:else}
  <TitleBar tone="plain" />
  <main class="plain">
    <UnlockScreen status={session.status} lockReason={session.lockReason} onunlocked={adopt} />
  </main>
{/if}

<style>
  .preview-banner {
    position: sticky;
    top: 0;
    z-index: 1;
    max-width: none;
    margin: 0;
    padding: var(--space-3) var(--space-6);
    border-bottom: var(--border-width) solid var(--colour-border-strong);
    /* The one place in the interface that uses the warning colour as a background. It is
     * meant to be impossible to mistake for part of the application. */
    background-color: var(--colour-warning);
    color: var(--colour-surface);
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    text-align: center;
    user-select: text;
  }

  /*
   * The screens that have no shell: creating the vault, a header that cannot be read, the
   * lock screen, the diagnostics. They fill what is left under the title bar, and they get
   * the title bar because a window with no system decoration that could not be moved or
   * closed until somebody typed a password would be a trap.
   */
  .plain {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--space-6);
    overflow: auto;
    padding: var(--space-6) var(--space-6) var(--space-8);
  }

  .muted {
    color: var(--colour-text-muted);
  }
</style>
