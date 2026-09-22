<script lang="ts">
  /**
   * Describing a habit, and saying what changing one would mean before it is changed.
   *
   * One screen for both jobs, because the fields are the same fields and two screens would be
   * two places to forget one of them. Which job it is doing is the presence of an identifier.
   *
   * Nothing here decides whether a habit is acceptable. The controls stop what a control can
   * stop — a number field takes no letters, a date field takes no half-dates — and the core
   * says what a habit may be. Two sets of rules end up disagreeing, and the one that loses is
   * always the one further from the data, so every refusal on this screen came from Rust and
   * is drawn beside the field the core named.
   *
   * The warning before saving is the one thing this screen does that the list does not. It is
   * asked for rather than worked out: `previewHabitUpdate` says what the edit would do to the
   * run, and the alternative would be counting a streak in JavaScript, which is the one thing
   * this module never does anywhere.
   */
  import { ipc } from '$ipc';
  import type { HabitDetail, UpdateImpact } from '../../lib/ipc.types';
  import ScreenHeader from '../../lib/shell/ScreenHeader.svelte';
  import { sectionOf } from '../../lib/shell/sections';
  import AsyncView from '../../lib/ui/AsyncView.svelte';
  import { load, loading, messageFor, ready, type Async } from '../../lib/ui/async';
  import {
    WEEKDAYS,
    blankForm,
    countsQuantity,
    draftOf,
    formOf,
    problemText,
    problemsByField,
    type FormState,
  } from './draft';
  import { daysText } from './detail';

  interface Props {
    /** Which habit is being edited, or nothing, which means one is being created. */
    id: string | null;
    /** Where to go when this is finished or abandoned. */
    onDone: (savedId: string | null) => void;
  }

  const { id, onDone }: Props = $props();

  /**
   * What date to put in the start field of a new habit, read from this machine's clock.
   *
   * The one place on this side that reads a calendar, and it is allowed to because of what
   * the value is for: a suggestion in a field somebody is looking at and can change, not a
   * day anything is written to. Being a day out at a time zone boundary means the field shows
   * a date next to the person's cursor; being a day out when marking would mean writing to
   * the wrong day silently, which is why that number comes from the core instead.
   */
  function suggestedStart(): number {
    const now = new Date();
    return now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate();
  }

  const section = sectionOf('habits');

  /**
   * Which of the four states the screen is in. What is being edited is `form`, below.
   *
   * Two variables rather than one, and it is a limit of the framework rather than a design:
   * a radio group cannot bind to a snippet parameter, so the thing every control writes into
   * has to be a variable of this component. `view` decides whether the form is drawn at all;
   * `form` is what it draws.
   */
  let view = $state<Async<FormState>>(loading());

  /** What is on screen. Read from the habit when editing, blank when creating. */
  let form = $state<FormState>(blankForm(suggestedStart()));

  /** What the core refused, filed under the field it named. Cleared on every attempt. */
  let problems = $state(new Map<string, string[]>());

  /** Whatever went wrong that was not about a field, said in one sentence. */
  let refusal = $state<string | null>(null);

  /** Whether a call is in flight, so the form is not sent twice by a double press. */
  let sending = $state(false);

  /** What saving would do, when the core says it would change what the run means. */
  let warning = $state<UpdateImpact | null>(null);

  /** The dialog's own element, for the focus trap and for putting the focus back. */
  let dialog = $state<HTMLDivElement | null>(null);
  let cameFrom: HTMLElement | null = null;

  $effect(() => {
    void (async () => {
      const next =
        id === null
          ? ready(blankForm(suggestedStart()))
          : await load(
              async () => formOf(await ipc.getHabit(id)),
              () => false,
            );
      if (next.status === 'ready') {
        form = next.value;
      }
      view = next;
    })();
  });

  /** The problems for one field, already turned into sentences. */
  function sentencesFor(field: string): string[] {
    return (problems.get(field) ?? []).map((code) => problemText(field, code));
  }

  /** Takes what the core refused and files it, or says the refusal in one line. */
  function refused(thrown: unknown): void {
    const error = thrown as { kind?: unknown; problems?: unknown };
    if (error.kind === 'invalid' && Array.isArray(error.problems)) {
      // The core names every field it objects to, not the first, so the whole form can be
      // marked in one round trip rather than one refusal at a time.
      problems = problemsByField(error.problems as { field: string; code: string }[]);
      refusal = null;
      return;
    }
    problems = new Map();
    refusal =
      typeof error.kind === 'string'
        ? messageFor(error as Parameters<typeof messageFor>[0])
        : 'No se ha podido guardar.';
  }

  /** Writes the draft, whether it is a new habit or an edit that has been confirmed. */
  async function save(): Promise<void> {
    sending = true;
    try {
      const draft = draftOf(form);
      const saved: HabitDetail =
        id === null ? await ipc.createHabit(draft) : (await ipc.updateHabit(id, draft)).habit;
      onDone(saved.id);
    } catch (thrown: unknown) {
      refused(thrown);
    } finally {
      sending = false;
    }
  }

  /**
   * Asks what saving would do, and saves straight away when the answer is «nothing unusual».
   *
   * Raising the target of a habit that counts a quantity comes through here and saves without
   * a word, because the days already marked keep the target they were judged against. Turning
   * a habit round, or taking days out of its schedule, does not.
   */
  async function submit(): Promise<void> {
    problems = new Map();
    refusal = null;
    if (id === null) {
      await save();
      return;
    }
    sending = true;
    try {
      const impact = await ipc.previewHabitUpdate(id, draftOf(form));
      if (impact.streakMeaningChanged) {
        cameFrom = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        warning = impact;
        return;
      }
    } catch (thrown: unknown) {
      refused(thrown);
      return;
    } finally {
      sending = false;
    }
    await save();
  }

  /** Closes the warning without changing anything, and puts the focus back where it was. */
  function cancelWarning(): void {
    warning = null;
    cameFrom?.focus();
  }

  /**
   * Keeps the focus inside the dialog while it is open, and lets Escape out.
   *
   * Escape is the same as cancelling, which is the promise a dialog makes: the way out that
   * costs nothing is the one that needs no aiming.
   */
  function trap(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.preventDefault();
      cancelWarning();
      return;
    }
    if (event.key !== 'Tab' || dialog === null) {
      return;
    }
    const focusable = dialog.querySelectorAll<HTMLElement>('button');
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (first === undefined || last === undefined) {
      return;
    }
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  $effect(() => {
    if (warning !== null) {
      dialog?.querySelector('button')?.focus();
    }
  });
</script>

<ScreenHeader
  {section}
  title={id === null ? 'Nuevo hábito' : 'Editar hábito'}
  lede={id === null
    ? 'Descríbelo como lo dirías en voz alta. Todo se puede cambiar después.'
    : undefined}
/>

<AsyncView state={view} label="Leyendo el hábito…">
  {#snippet empty()}
    <p>Ese hábito ya no está.</p>
  {/snippet}

  {#snippet ready()}
    <form
      class="habit-form"
      novalidate
      onsubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <div class="field">
        <label for="habit-name">Nombre</label>
        <input
          id="habit-name"
          type="text"
          bind:value={form.name}
          aria-describedby={sentencesFor('name').length > 0 ? 'habit-name-problem' : undefined}
        />
        {#if sentencesFor('name').length > 0}
          <div class="problems" id="habit-name-problem">
            {#each sentencesFor('name') as said, at (at)}
              <p class="problem">{said}</p>
            {/each}
          </div>
        {/if}
      </div>

      <div class="field">
        <label for="habit-notes">Notas</label>
        <textarea id="habit-notes" rows="3" bind:value={form.notes}></textarea>
        <p class="help">Para ti. No sale en la lista de hoy, solo dentro del hábito.</p>
      </div>

      <fieldset class="field">
        <legend>Cada cuánto se juzga</legend>
        <label class="choice">
          <input type="radio" value="daily" bind:group={form.period} />
          Por día
        </label>
        <label class="choice">
          <input type="radio" value="weekly" bind:group={form.period} />
          Por semana
        </label>
      </fieldset>

      <fieldset class="field">
        <legend>Cómo se lee</legend>
        <label class="choice">
          <input type="radio" value="atLeast" bind:group={form.direction} />
          Quiero hacerlo, al menos
        </label>
        <label class="choice">
          <input type="radio" value="atMost" bind:group={form.direction} />
          Quiero evitarlo, como mucho
        </label>
        <p class="help">
          Si lo quieres evitar, el día bueno es el día que no marcas: marcar es apuntar una recaída.
        </p>
      </fieldset>

      <div class="field">
        <label for="habit-unit">Unidad</label>
        <input id="habit-unit" type="text" bind:value={form.unit} />
        <p class="help">
          Ponle una unidad —ml, páginas, minutos— si quieres contar una cantidad. Déjalo vacío si
          solo se hace o no se hace.
        </p>
      </div>

      <!--
        The three fields that only mean something for a habit counting a quantity are removed
        rather than disabled. A disabled control is a control somebody tries to use and then
        wonders what they did wrong; an absent one is a question that was never asked.
      -->
      {#if countsQuantity(form) || form.period === 'weekly'}
        <div class="field">
          <label for="habit-target">
            {countsQuantity(form) ? 'Objetivo por período' : 'Días por semana'}
          </label>
          <input
            id="habit-target"
            type="number"
            min="0"
            inputmode="numeric"
            bind:value={form.target}
            aria-describedby={sentencesFor('target').length > 0
              ? 'habit-target-problem'
              : undefined}
          />
          {#if sentencesFor('target').length > 0}
            <div class="problems" id="habit-target-problem">
              {#each sentencesFor('target') as said, at (at)}
                <p class="problem">{said}</p>
              {/each}
            </div>
          {/if}
        </div>
      {/if}

      {#if countsQuantity(form)}
        <fieldset class="field">
          <legend>Cómo se juntan las marcas del período</legend>
          <label class="choice">
            <input type="radio" value="sum" bind:group={form.aggregation} />
            Sumándolas
          </label>
          <label class="choice">
            <input type="radio" value="highest" bind:group={form.aggregation} />
            Quedándose con la mayor
          </label>
        </fieldset>
      {/if}

      <fieldset class="field">
        <legend>Qué días toca</legend>
        <div class="days">
          {#each WEEKDAYS as day, at (day)}
            <label class="choice">
              <input type="checkbox" bind:checked={form.days[at]} />
              {day}
            </label>
          {/each}
        </div>
        <!-- Said out loud rather than left to be worked out from an empty row of boxes. -->
        <p class="help">
          {form.days.some((on) => on)
            ? 'Los días que no marques no cuentan ni a favor ni en contra.'
            : 'Sin ninguno marcado, el hábito toca todos los días.'}
        </p>
      </fieldset>

      <div class="field">
        <label for="habit-started">Desde cuándo</label>
        <input
          id="habit-started"
          type="date"
          bind:value={form.startedOn}
          aria-describedby="habit-started-help"
        />
        <p class="help" id="habit-started-help">
          Puede ser muy anterior a hoy: es como se declara un hábito que ya venías haciendo. Los
          días antes de esta fecha no se juzgan.
        </p>
        {#each sentencesFor('startedOn') as said, at (at)}
          <p class="problem">{said}</p>
        {/each}
      </div>

      {#if refusal !== null}
        <p class="problem refusal">{refusal}</p>
      {/if}

      <div class="submit">
        <button type="submit" class="primary" disabled={sending}>
          {id === null ? 'Crear el hábito' : 'Guardar los cambios'}
        </button>
        <button
          type="button"
          onclick={() => {
            onDone(null);
          }}>Cancelar</button
        >
      </div>
    </form>

    {#if warning !== null}
      <!--
        A dialog, which is the one thing on this screen that is one, because this is the one
        choice here that carrying on cannot undo: the run on the screen would start being
        counted by different rules.
      -->
      <div class="veil">
        <div
          class="dialog"
          role="dialog"
          aria-modal="true"
          aria-labelledby="warning-title"
          bind:this={dialog}
          onkeydown={trap}
          tabindex="-1"
        >
          <h2 id="warning-title">Esto cambia lo que la racha significa</h2>

          <p>
            Con las reglas que tiene ahora, la racha es de {daysText(warning.currentStreakBefore)}.
            Con las nuevas sería de {daysText(warning.currentStreakAfter)}.
          </p>

          {#if warning.entriesOutsideNewSchedule > 0}
            <p>
              {warning.entriesOutsideNewSchedule === 1
                ? 'Queda 1 marca en un día que el horario nuevo no incluye.'
                : `Quedan ${String(warning.entriesOutsideNewSchedule)} marcas en días que el horario nuevo no incluye.`}
              No se borran, pero dejan de contar.
            </p>
          {/if}

          <p>No se ha guardado nada todavía.</p>

          <div class="submit">
            <button
              type="button"
              class="primary"
              onclick={() => {
                warning = null;
                void save();
              }}>Guardar de todos modos</button
            >
            <button type="button" onclick={cancelWarning}>Cancelar</button>
          </div>
        </div>
      </div>
    {/if}
  {/snippet}
</AsyncView>

<style>
  .habit-form {
    display: flex;
    max-width: var(--measure);
    flex-direction: column;
    gap: var(--space-5);
    margin-top: var(--space-6);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }

  label,
  legend {
    padding: 0;
    color: var(--colour-text-muted);
    font-size: var(--text-label);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
  }

  /* A choice is a word with a box in front of it, in ordinary case: it is read, not scanned. */
  .choice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--colour-text);
    font-size: var(--text-base);
    font-weight: var(--weight-regular);
    letter-spacing: normal;
    text-transform: none;
  }

  input[type='text'],
  input[type='number'],
  input[type='date'],
  textarea {
    padding: var(--space-3);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: var(--colour-surface-sunken);
    color: var(--colour-text);
    font: inherit;
  }

  .days {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .help {
    margin: 0;
    color: var(--colour-text-muted);
    font-size: var(--text-sm);
  }

  /* Under the field it belongs to, never in a column above the form. */
  /*
   * One box per field, holding every complaint the core made about it.
   *
   * The box exists so that the identifier `aria-describedby` points at is written once. A
   * field the core objects to twice used to render the same identifier twice, which is an
   * invalid document and a description that names an element the browser has to pick
   * between.
   */
  .problems {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .problem {
    margin: 0;
    color: var(--colour-negative);
    font-size: var(--text-sm);
  }

  .refusal {
    padding: var(--space-3) var(--space-4);
    border-left: var(--rule-width) solid var(--colour-negative);
    background-color: var(--colour-surface-sunken);
    color: var(--colour-text);
  }

  .submit {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .submit button {
    padding: var(--space-3) var(--space-4);
    border: var(--border-width) solid var(--colour-border-strong);
    border-radius: var(--radius-sm);
    background-color: var(--colour-surface-raised);
    color: var(--colour-text);
    font: inherit;
  }

  /* One accent per screen, and it is whichever button finishes the job. */
  .submit .primary {
    border-color: var(--colour-accent);
    background-color: var(--colour-accent);
    color: var(--colour-accent-contrast);
    font-weight: var(--weight-semibold);
  }

  .submit button:disabled {
    border-color: var(--colour-border);
    background-color: var(--colour-surface-sunken);
    color: var(--colour-text-faint);
  }

  /*
   * A dialog and not a layer, because this is the one choice on this screen that carrying on
   * cannot undo. It is centred over the window and the window behind it is not tinted: a tint
   * would be a colour outside the token set and a second thing competing for the eye.
   */
  .veil {
    position: fixed;
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-5);
    inset: 0;
  }

  .dialog {
    max-width: var(--measure);
    padding: var(--space-6);
    border: var(--border-width) solid var(--colour-warning);
    border-top: var(--card-edge-width) solid var(--colour-warning);
    border-radius: var(--radius-md);
    background-color: var(--colour-surface-raised);
  }

  .dialog h2 {
    margin: 0 0 var(--space-4);
    color: var(--colour-warning);
    font-size: var(--text-lg);
  }

  .dialog p {
    margin: 0 0 var(--space-4);
    max-width: var(--measure);
  }
</style>
