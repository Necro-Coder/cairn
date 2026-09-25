/**
 * A stand-in for the core, so that the interface can be opened in an ordinary browser.
 *
 * There is no Rust behind a browser tab. Without this module the screens would show nothing
 * but an error, which makes the interface impossible to look at anywhere except inside the
 * desktop window, and in particular impossible to look at on a phone before there is an
 * application to install on one.
 *
 * What this module is not, and what nothing here can be used to claim:
 *
 * - It validates nothing about the core. There is no Rust, no key derivation, no encryption
 *   and no database. A screen that works here can still be broken in the real application.
 * - **Nothing here is encrypted and nothing here is a secret.** The password is held as a
 *   plain string and compared with `===`. That is not a shortcut to be tidied up later; it is
 *   the reason this file may never reach a production bundle, and the gate that searches the
 *   built artefact for the marker below is what enforces it.
 * - The numbers it returns are invented. Reading a performance budget off a preview is
 *   reading a number somebody typed into this file.
 * - It is not a test double. Tests run against the real boundary or against Rust.
 *
 * Every value is fixed and deterministic. Random data, even seeded, hides intermittent
 * mistakes and means two screenshots of the same screen cannot be compared. The one thing
 * that does change is the clock, because a countdown that never counts cannot be looked at.
 *
 * This file is never in a production build. The `$ipc` alias only points here when Vite
 * runs in preview mode, so it is not behind a condition at runtime, it is absent from the
 * module graph entirely. A gate in the pipeline searches the built bundle for the marker
 * below and fails if it finds it, because a build flag on its own is not a defence: one
 * badly placed import undoes it without saying anything.
 */

import type {
  AppInfo,
  BackupError,
  BackupProgress,
  Diagnostics,
  InactivityChoice,
  InstanceState,
  IpcSurface,
  KdfParams,
  KdfReport,
  LockReason,
  PasswordStrength,
  SampleError,
  SampleHabit,
  VaultCondition,
  VaultError,
  VaultStatus,
  DayState,
  FieldProblem,
  HabitDetail,
  HabitDraft,
  HabitsError,
  HabitSummary,
  Streak,
  WeekProgress,
} from '../ipc.types';

/**
 * The marker, shown to the person and searched for in the built bundle.
 *
 * One string with two jobs on purpose. Shown on screen it is unmistakable in a way that
 * a tasteful grey label is not, and a machine token in the middle of an interface is
 * exactly the kind of thing nobody mistakes for the real application. Searched for in
 * `dist/`, it is proof about the artefact rather than about the intention behind it.
 *
 * Because it is one string used in both places, the banner cannot say one thing while the
 * gate looks for another.
 */
const MARKER = 'CAIRN-PREVIEW-MOCK-DATA';

/** Obviously invented, and obviously not a version anybody released. */
const PREVIEW_APP_INFO: AppInfo = {
  name: 'Cairn (datos de mentira)',
  version: '0.0.0-ejemplo',
  profile: 'debug',
};

/** Obviously invented. No real system reports an architecture called this. */
const PREVIEW_DIAGNOSTICS: Diagnostics = {
  app: PREVIEW_APP_INFO,
  os: 'sistema de ejemplo',
  arch: 'arquitectura de ejemplo',
  webviewVersion: 'navegador de ejemplo',
  database: { state: 'open', schemaVersion: 2, tombstones: 1 },
  uptimeMs: 1234,
};

/**
 * The sample habits this stand-in pretends are in a database.
 *
 * Fixed rather than generated, like everything else here, so that two screenshots of the
 * diagnostics screen can be compared. One of them is a tombstone, because a list where nothing
 * has been deleted is a list that never shows how a deleted row is drawn.
 */
const previewHabits: SampleHabit[] = [
  {
    id: '00000000-0000-4000-8000-000000000001',
    name: 'Hábito de prueba',
    deleted: false,
    cursor: '0'.repeat(32),
  },
  {
    id: '00000000-0000-4000-8000-000000000002',
    name: 'Hábito de prueba',
    deleted: true,
    cursor: '1'.repeat(32),
  },
];

/** How many sample habits this stand-in has invented, so each one gets its own identifier. */
let previewHabitCount = previewHabits.length;

/** The fewest characters the real policy accepts. Kept in step with `cairn-domain`. */
const MIN_PASSWORD_CHARS = 12;

/** How long each inactivity choice lasts, in seconds. `never` has no entry. */
const INACTIVITY_SECONDS: Partial<Record<InactivityChoice, number>> = {
  one: 60,
  five: 300,
  fifteen: 900,
  thirty: 1800,
};

/** The longest the backoff grows to, matching the real schedule. */
const MAX_BACKOFF_S = 300;

/**
 * Everything the stand-in pretends to remember.
 *
 * None of it is stored anywhere. Reloading the page is a fresh machine with no vault, which
 * is the honest behaviour for something that keeps its state in a variable.
 */
interface PreviewVault {
  password: string;
  unlocked: boolean;
  failedAttempts: number;
  lockedUntilMs: number;
  lastActivityMs: number;
  inactivity: InactivityChoice;
  kdf: KdfReport;
  condition: VaultCondition;
}

let vault: PreviewVault | null = null;

/**
 * Whether the pretend window is maximised.
 *
 * A browser tab has no such state, so this exists only so that the button in the header
 * draws both of its glyphs when somebody presses it.
 */
let maximised = false;

/** The listeners the interface has registered for the lock event. */
const lockListeners = new Set<(reason: LockReason) => void>();

/**
 * The listeners registered for export and verification progress.
 *
 * Kept, and never called. Nothing here reads a file, so there is no progress to report; the
 * set exists so that subscribing and unsubscribing behave the way they will in the window.
 */
const progressListeners = new Set<(progress: BackupProgress) => void>();

/** Tells every listener the vault has closed. */
function announceLock(reason: LockReason): void {
  for (const listener of lockListeners) {
    listener(reason);
  }
}

/**
 * Fails the way the real boundary fails, with the tagged object rather than a sentence.
 *
 * Not an `Error`, deliberately. Tauri rejects a command with whatever the core serialised,
 * which is this object, and a stand-in that rejected with something else would let a screen
 * be written against a shape the real boundary never produces.
 */
function reject(error: VaultError | SampleError): Promise<never> {
  // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- the real boundary rejects with exactly this plain tagged object, and a stand-in that wrapped it in an Error would let a screen be written against a shape the core never produces
  return Promise.reject(error);
}

/** The same, for the backup commands, which have an error set of their own. */
function rejectBackup(error: BackupError): Promise<never> {
  // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see the note on `reject` above; the reason is the same one
  return Promise.reject(error);
}

/** The same doubling schedule the core uses, capped in the same place. */
function backoffSeconds(failedAttempts: number): number {
  if (failedAttempts <= 0) {
    return 0;
  }
  return Math.min(2 ** (failedAttempts - 1), MAX_BACKOFF_S);
}

/** Seconds left of a wait that ends at `untilMs`, rounded up as the core rounds it. */
function remainingSeconds(untilMs: number): number {
  const left = untilMs - Date.now();
  return left <= 0 ? 0 : Math.min(Math.ceil(left / 1000), MAX_BACKOFF_S);
}

/** Closes the vault if its own clock says to, and says so, exactly as the watchdog does. */
function applyInactivity(): void {
  if (vault === null || !vault.unlocked) {
    return;
  }

  const seconds = INACTIVITY_SECONDS[vault.inactivity];
  if (seconds === undefined) {
    return;
  }

  if (Date.now() - vault.lastActivityMs >= seconds * 1000) {
    vault.unlocked = false;
    announceLock('inactivity');
  }
}

/**
 * Which instance state the preview reports.
 *
 * A browser tab is never a second copy of anything: there is no lock, no data directory and no
 * other process, so the honest answer is always `held`. The query parameter exists because the
 * screen that explains a refusal is otherwise unreachable here, and a screen nobody can reach is
 * a screen nobody checks for contrast, focus order or a reader.
 *
 * Anything that is not one of the three refusals is `held`, so a value typed into the address
 * bar cannot put the preview into a state the real application has no name for.
 */
function previewInstanceState(): InstanceState {
  const asked = new URLSearchParams(globalThis.location.search).get('instance');

  return asked === 'alreadyRunning' || asked === 'unavailable' || asked === 'noDirectory'
    ? asked
    : 'held';
}

/** What the interface is told, assembled the way the core assembles it. */
function status(): VaultStatus {
  applyInactivity();

  if (vault === null) {
    return {
      exists: false,
      unlocked: false,
      condition: 'noVaultYet',
      kdf: null,
      failedAttempts: 0,
      lockedOutForS: 0,
      inactivity: 'five',
      idleRemainingS: null,
    };
  }

  const seconds = INACTIVITY_SECONDS[vault.inactivity];
  const idleRemainingS =
    vault.unlocked && seconds !== undefined
      ? Math.max(0, Math.ceil((vault.lastActivityMs + seconds * 1000 - Date.now()) / 1000))
      : null;

  return {
    exists: true,
    unlocked: vault.unlocked,
    condition: vault.condition,
    kdf: vault.kdf,
    failedAttempts: vault.failedAttempts,
    lockedOutForS: remainingSeconds(vault.lockedUntilMs),
    inactivity: vault.inactivity,
    idleRemainingS,
  };
}

/** The same two length rules the real policy applies, and no composition rules either. */
function validate(password: string): VaultError | null {
  const chars = [...password].length;
  if (chars < MIN_PASSWORD_CHARS) {
    return { kind: 'passwordTooShort', chars, min: MIN_PASSWORD_CHARS };
  }
  return null;
}

/** A report shaped like the one the core sends, from the parameters it was asked for. */
function report(params: KdfParams): KdfReport {
  return {
    memoryKib: params.memoryKib,
    passes: params.passes,
    lanes: params.lanes,
    writtenAtUs: Date.now() * 1000,
  };
}

/* -----------------------------------------------------------------------------------------
 * Habits.
 *
 * The one part of this stand-in that has to behave rather than merely answer: a screen where
 * marking a day does nothing cannot be used to judge whether marking a day feels right, which
 * is the whole reason for looking at an interface in a browser.
 *
 * So there is a small judge below. It is **not** the domain. It classifies a square from the
 * marks this file holds in a variable, by rules written here and nowhere else, and it exists
 * so that pressing a key redraws something. What a day really is, is decided in Rust, and a
 * disagreement between the two is a mistake in this file and never evidence about the core.
 *
 * The one thing derived from the real clock is which day is today, for the same reason the
 * countdown is: a calendar whose today is a day in the past cannot be marked, and the screen
 * being looked at is the one where somebody marks today. Everything else is fixed, and two
 * screenshots taken on the same day are identical.
 * -------------------------------------------------------------------------------------- */

/** Every day of the week, as the mask spells it: bit 0 is Monday. */
const EVERY_DAY = 0b111_1111;

/** Monday, Wednesday and Friday. Bits 0, 2 and 4. */
const MON_WED_FRI = 0b001_0101;

/** How far back the stand-in bothers to invent marks. Two years is more than a screen draws. */
const SEEDED_DAYS = 760;

/** A day as `YYYYMMDD`, which is how a day travels on this boundary. */
type DayNumber = number;

/** The `YYYYMMDD` of a date, read in UTC so no machine's zone changes what is drawn. */
function dayNumberOf(date: Date): DayNumber {
  return date.getUTCFullYear() * 10_000 + (date.getUTCMonth() + 1) * 100 + date.getUTCDate();
}

/** The date one `YYYYMMDD` names, at midnight UTC. */
function dateOf(day: DayNumber): Date {
  const year = Math.trunc(day / 10_000);
  const month = Math.trunc(day / 100) % 100;
  return new Date(Date.UTC(year, month - 1, day % 100));
}

/** The day `offset` days away from another one, negative for earlier. */
function dayPlus(day: DayNumber, offset: number): DayNumber {
  const moved = dateOf(day);
  moved.setUTCDate(moved.getUTCDate() + offset);
  return dayNumberOf(moved);
}

/** Which bit of the mask a day answers to: Monday is 0 and Sunday is 6. */
function weekdayIndex(day: DayNumber): number {
  return (dateOf(day).getUTCDay() + 6) % 7;
}

/** Whether the mask includes that weekday. Zero means every day, as the core reads it. */
function isScheduled(mask: number, day: DayNumber): boolean {
  const effective = mask === 0 ? EVERY_DAY : mask;
  return (effective & (1 << weekdayIndex(day))) !== 0;
}

/** The Monday of the week a day falls in. */
function mondayOf(day: DayNumber): DayNumber {
  return dayPlus(day, -weekdayIndex(day));
}

/** Today, read on every call so a tab left open overnight is not stuck on yesterday. */
function today(): DayNumber {
  return dayNumberOf(new Date());
}

/** Every day of one year, oldest first. */
function daysOfYear(year: number): DayNumber[] {
  const days: DayNumber[] = [];
  const last = year * 10_000 + 1231;
  for (let day = year * 10_000 + 101; day <= last; day = dayPlus(day, 1)) {
    days.push(day);
  }
  return days;
}

/** What a habit is, minus the four fields worked out rather than stored. */
type HabitSeed = Omit<HabitDetail, 'today' | 'todayDay' | 'streak' | 'archived' | 'position'>;

/**
 * One habit as this file remembers it: what the boundary carries, and the marks.
 *
 * The marks are a map from day to amount, so toggling one is a write to a single key and a
 * whole year can be judged without searching a list three hundred and sixty-five times.
 */
interface PreviewHabit {
  detail: HabitSeed;
  archived: boolean;
  position: number;
  marks: Map<DayNumber, number>;
}

/**
 * Whether a habit counts a quantity, which is what naming a unit makes it do.
 *
 * The same question the core asks, asked the same way: the unit is what decides, not the
 * target. A weekly habit with no unit has a target too, and that target is how many days of
 * the week it wants rather than how much any one of them wants.
 */
function countsQuantity(habit: PreviewHabit): boolean {
  return habit.detail.unit !== null;
}

/**
 * What one day asked for.
 *
 * A habit that counts a quantity asks for its target. A habit that is simply done or not
 * still has a number behind it, and which number depends on which way it is read: one thing
 * done, or nothing done at all.
 */
function dayTarget(habit: PreviewHabit): number {
  if (countsQuantity(habit)) {
    return habit.detail.target ?? 1;
  }
  return habit.detail.direction === 'atLeast' ? 1 : 0;
}

/**
 * Whether the day did what was asked of it.
 *
 * The two directions are not mirror images, and the asymmetry is the whole point. A habit
 * being built needs a mark to have been met: an absent row means nothing happened. A habit
 * being cut down is met by the absence itself, which is what "did not smoke today" is.
 */
function meets(habit: PreviewHabit, amount: number, marked: boolean): boolean {
  return habit.detail.direction === 'atLeast'
    ? marked && amount >= dayTarget(habit)
    : amount <= dayTarget(habit);
}

/**
 * What one square says.
 *
 * The five variants and the order they are decided in, mirroring the core's own. Only a
 * habit somebody is building can be overshot on a day off: not smoking on a day the habit
 * never asked about is an ordinary day, and calling it extra would hand out credit for
 * doing nothing at all.
 */
function judge(habit: PreviewHabit, day: DayNumber): DayState {
  if (day < habit.detail.startedOn || day > today()) {
    return { state: 'noData' };
  }
  const amount = habit.marks.get(day) ?? 0;
  const met = meets(habit, amount, habit.marks.has(day));
  const target = dayTarget(habit);
  if (isScheduled(habit.detail.scheduleMask, day)) {
    return met ? { state: 'done', amount, target } : { state: 'missed', amount, target };
  }
  return met && habit.detail.direction === 'atLeast'
    ? { state: 'extra', amount, target }
    : { state: 'notScheduled', amount };
}

/** How many scheduled days in a row, walking back, were met. Today counts only if it was. */
function streakOf(habit: PreviewHabit): number {
  let days = 0;
  let day = today();
  for (let step = 0; step < SEEDED_DAYS; step += 1) {
    if (day < habit.detail.startedOn) {
      break;
    }
    if (isScheduled(habit.detail.scheduleMask, day)) {
      if (judge(habit, day).state !== 'done') {
        // Today unmarked does not end the run, it puts it at risk: the day is not over yet.
        // Any earlier day unmarked does end it, which is the whole difference between the two.
        if (step > 0) {
          break;
        }
      } else {
        days += 1;
      }
    }
    day = dayPlus(day, -1);
  }
  return days;
}

/** How the week holding today is going, for a habit judged by the week, and nothing otherwise. */
function weekProgressOf(habit: PreviewHabit): WeekProgress | null {
  if (habit.detail.period !== 'weekly') {
    return null;
  }
  const monday = mondayOf(today());
  let done = 0;
  for (let step = 0; step < 7; step += 1) {
    if (judge(habit, dayPlus(monday, step)).state === 'done') {
      done += 1;
    }
  }
  return { done, target: habit.detail.target ?? 0 };
}

/** The run as it stands, and whether today is still open with nothing marked on it. */
function streakDtoOf(habit: PreviewHabit): Streak {
  const days = streakOf(habit);
  const now = today();
  const atRisk =
    days > 0 && isScheduled(habit.detail.scheduleMask, now) && judge(habit, now).state !== 'done';
  return { days, atRisk, weekProgress: weekProgressOf(habit) };
}

/** One habit in the shape the list carries: no note, ever. */
function summaryOf(habit: PreviewHabit): HabitSummary {
  return {
    id: habit.detail.id,
    name: habit.detail.name,
    icon: habit.detail.icon,
    color: habit.detail.color,
    period: habit.detail.period,
    unit: habit.detail.unit,
    target: habit.detail.target,
    direction: habit.detail.direction,
    scheduleMask: habit.detail.scheduleMask === 0 ? EVERY_DAY : habit.detail.scheduleMask,
    position: habit.position,
    archived: habit.archived,
    todayDay: today(),
    today: judge(habit, today()),
    streak: streakDtoOf(habit),
  };
}

/** One habit in full. */
function detailOf(habit: PreviewHabit): HabitDetail {
  return {
    ...summaryOf(habit),
    notes: habit.detail.notes,
    startedOn: habit.detail.startedOn,
    aggregation: habit.detail.aggregation,
  };
}

/** Marks a run of days back from one, one mark per day the schedule includes. */
function runBackFrom(
  start: DayNumber,
  scheduledDays: number,
  mask: number,
  amount: number,
): Array<readonly [DayNumber, number]> {
  const marks: Array<readonly [DayNumber, number]> = [];
  let day = start;
  while (marks.length < scheduledDays) {
    if (isScheduled(mask, day)) {
      marks.push([day, amount]);
    }
    day = dayPlus(day, -1);
  }
  return marks;
}

/**
 * Every third scheduled day left unmarked, going back two years.
 *
 * This is what puts all five squares into one year of one habit: the days the schedule
 * includes and this keeps give `done`, the ones it skips give `missed`, the days the schedule
 * never includes give `notScheduled`, the Sunday added by hand below gives `extra`, and the
 * rest of the year after today gives `noData`.
 */
function patternedMarks(mask: number): Array<readonly [DayNumber, number]> {
  const marks: Array<readonly [DayNumber, number]> = [];
  let kept = 0;
  for (let step = 0; step < SEEDED_DAYS; step += 1) {
    const day = dayPlus(today(), -step);
    if (!isScheduled(mask, day)) {
      continue;
    }
    kept += 1;
    if (kept % 3 !== 0) {
      marks.push([day, 1]);
    }
  }
  return marks;
}

/** Builds a habit for the list below, so each entry says only what makes it different. */
function makeHabit(
  seed: HabitSeed,
  position: number,
  options: { archived?: boolean; marks?: Iterable<readonly [DayNumber, number]> } = {},
): PreviewHabit {
  return {
    detail: seed,
    archived: options.archived ?? false,
    position,
    marks: new Map(options.marks ?? []),
  };
}

/**
 * The fixed set: one of each thing a screen has to be able to draw.
 *
 * A run about to be broken, a habit to be avoided, a habit counted by the week, a habit
 * counted in millilitres, and one put away so the other list is not empty. An empty list is
 * reached by deleting them, which this stand-in performs for real, so the empty screen needs
 * no switch of its own to be looked at.
 */
function seedHabits(): PreviewHabit[] {
  const now = today();
  const longAgo = dayPlus(now, -700);
  const thisMonday = mondayOf(now);
  return [
    makeHabit(
      {
        id: '00000000-0000-4000-8000-00000000a001',
        name: 'Meditar',
        icon: null,
        color: null,
        period: 'daily',
        unit: null,
        target: null,
        direction: 'atLeast',
        scheduleMask: EVERY_DAY,
        notes: 'Diez minutos. Cuenta sentarse, no cuánto sale.',
        startedOn: longAgo,
        aggregation: 'sum',
      },
      0,
      // Twelve days behind today and nothing on today: the run is alive and at risk.
      { marks: runBackFrom(dayPlus(now, -1), 12, EVERY_DAY, 1) },
    ),
    makeHabit(
      {
        id: '00000000-0000-4000-8000-00000000a002',
        name: 'Sin azúcar añadido',
        icon: null,
        color: null,
        period: 'daily',
        unit: null,
        target: null,
        direction: 'atMost',
        scheduleMask: EVERY_DAY,
        notes: null,
        startedOn: longAgo,
        aggregation: 'sum',
      },
      1,
      // A habit to be avoided is read the other way round: the day nobody marked is the good
      // one, and a mark is a slip. So what is seeded is two slips, far enough back that the
      // run since the last one is worth showing, and nothing at all on today.
      { marks: [[dayPlus(now, -7), 1] as const, [dayPlus(now, -31), 1] as const] },
    ),
    makeHabit(
      {
        id: '00000000-0000-4000-8000-00000000a003',
        name: 'Correr',
        icon: null,
        color: null,
        period: 'weekly',
        unit: null,
        target: 3,
        direction: 'atLeast',
        scheduleMask: MON_WED_FRI,
        notes: null,
        startedOn: longAgo,
        aggregation: 'sum',
      },
      2,
      {
        marks: [
          ...patternedMarks(MON_WED_FRI).filter(([day]) => day < thisMonday),
          // The week in progress at one of three, and one Sunday nobody asked for.
          [thisMonday, 1],
          [dayPlus(thisMonday, -1), 1],
        ],
      },
    ),
    makeHabit(
      {
        id: '00000000-0000-4000-8000-00000000a004',
        name: 'Beber agua',
        icon: null,
        color: null,
        period: 'daily',
        unit: 'ml',
        target: 2000,
        direction: 'atLeast',
        scheduleMask: EVERY_DAY,
        notes: null,
        startedOn: longAgo,
        aggregation: 'sum',
      },
      3,
      // Today short of the target, and the four days before it met.
      { marks: [[now, 1500], ...runBackFrom(dayPlus(now, -1), 4, EVERY_DAY, 2000)] },
    ),
    makeHabit(
      {
        id: '00000000-0000-4000-8000-00000000a005',
        name: 'Leer antes de dormir',
        icon: null,
        color: null,
        period: 'daily',
        unit: null,
        target: null,
        direction: 'atLeast',
        scheduleMask: EVERY_DAY,
        notes: 'Guardado en su día. Está aquí para poder mirar la lista de archivados.',
        startedOn: dayPlus(now, -400),
        aggregation: 'sum',
      },
      4,
      { archived: true, marks: runBackFrom(dayPlus(now, -300), 20, EVERY_DAY, 1) },
    ),
  ];
}

/**
 * What was asked for in the address, if anything.
 *
 * Two switches, and both exist because the screen they show cannot be reached by pressing
 * anything. Read once, at load, so nothing changes under a screen already drawn, and guarded
 * because this module is imported by `node --test` as well as by a browser, and a test runner
 * has no address bar.
 */
const ASKED =
  typeof globalThis.location === 'undefined'
    ? new URLSearchParams()
    : new URLSearchParams(globalThis.location.search);

/**
 * Whether this preview was opened asking to see the screens with nothing on them.
 *
 * `?sin-habitos` in the address. The first day of a list is a real screen that somebody has to
 * be able to look at, and it is otherwise reached only by deleting every habit, which the
 * interface cannot do until the screen that archives and deletes exists.
 */
const ASKED_FOR_EMPTY = ASKED.has('sin-habitos');

/**
 * The largest list this stand-in will invent, whatever the address says.
 *
 * The number comes from an address bar, which is the same kind of input as anything else that
 * arrives from outside: a missing ceiling here is a tab that allocates until it dies, and a
 * preview that dies is indistinguishable from the fault this ceiling exists to let anybody
 * reproduce. Chosen to sit just above the ten thousand rows per table that the core's own
 * seeding command will write, so the worst list the diagnostics screen can actually produce is
 * one this can still be pointed at.
 */
const MAX_INVENTED = 12_000;

/**
 * How many habits to invent, from `?muchos=N` in the address, or none.
 *
 * A long list is the one thing the fixtures cannot show and the one thing that broke: a screen
 * that walks its whole list once per row is fast on the five habits below and unusable on a
 * thousand, and neither this file nor a unit test can tell the difference. Anything that is not
 * a whole number above zero is read as nothing asked for, rather than as an error, because an
 * address typed by hand is not a contract.
 */
const ASKED_FOR_MANY = ((): number => {
  const asked = Number(ASKED.get('muchos'));
  if (!Number.isInteger(asked) || asked <= 0) {
    return 0;
  }
  return Math.min(asked, MAX_INVENTED);
})();

/** What the diagnostics screen calls the habits it writes, which is what a sweep removes. */
const SAMPLE_NAME = 'Hábito de prueba';

/**
 * Whether a name is one of the stand-in's own, matched the way the core matches it.
 *
 * The name on its own, or the name followed by a space and a whole number. Written here as well
 * as in Rust because this file has to be able to show the screen after a sweep, and a looser
 * rule on this side would show a sweep taking habits the real one leaves alone.
 */
function isSampleName(name: string): boolean {
  if (name === SAMPLE_NAME) {
    return true;
  }
  const rest = name.startsWith(`${SAMPLE_NAME} `) ? name.slice(SAMPLE_NAME.length + 1) : null;
  return rest !== null && rest.length > 0 && /^\d+$/.test(rest);
}

/**
 * That many copies of the first fixture, each with its own identifier, name and place.
 *
 * Copies of one habit rather than a spread of the five: what a long list is for is the cost of
 * drawing it, and a habit that is a copy costs exactly what the original costs.
 */
function manyHabits(count: number): PreviewHabit[] {
  const [first] = seedHabits();
  if (first === undefined) {
    return [];
  }

  return Array.from({ length: count }, (_nothing, at) =>
    makeHabit(
      {
        ...first.detail,
        id: `00000000-0000-4000-8000-c${String(at).padStart(11, '0')}`,
        name: `${SAMPLE_NAME} ${String(at)}`,
      },
      at,
      { marks: first.marks },
    ),
  );
}

/** Everything the stand-in pretends is in a database. Reloading the page brings it back. */
const habits: PreviewHabit[] = ASKED_FOR_EMPTY
  ? []
  : ASKED_FOR_MANY > 0
    ? manyHabits(ASKED_FOR_MANY)
    : seedHabits();

/** How many habits this stand-in has invented, so each new one gets its own identifier. */
let inventedHabits = 0;

/** Fails the way the real boundary fails, with the tagged object rather than a sentence. */
function rejectHabits(error: HabitsError): Promise<never> {
  // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see the note on `reject` above; the reason is the same one
  return Promise.reject(error);
}

/** The habit with that identifier, or nothing. */
function findHabit(id: string): PreviewHabit | undefined {
  return habits.find((habit) => habit.detail.id === id);
}

/**
 * The one thing this stand-in refuses, and the reason it refuses it.
 *
 * Not a validator. The core owns what a habit may be and checks every field; this checks the
 * single one whose error the form has to be able to show, so that the state exists to look
 * at. Anything else typed into the form is accepted here and would not be in the window.
 */
function problemsWith(draft: HabitDraft): FieldProblem[] {
  return draft.name.trim() === '' ? [{ field: 'name', code: 'empty' }] : [];
}

/** The mask as the core stores it: zero arrives from a form and means every day. */
function normalisedMask(mask: number): number {
  return mask === 0 ? EVERY_DAY : mask;
}

/** Writes a draft over a habit, keeping its marks, its place and whether it is put away. */
function applyDraft(habit: PreviewHabit, draft: HabitDraft): void {
  habit.detail = {
    ...habit.detail,
    name: draft.name,
    notes: draft.notes,
    icon: draft.icon,
    color: draft.color,
    period: draft.period,
    unit: draft.unit,
    target: draft.target,
    aggregation: draft.aggregation,
    direction: draft.direction,
    scheduleMask: normalisedMask(draft.scheduleMask),
    startedOn: draft.startedOn,
  };
}

/** Whether saving that draft would change the rules the run is counted by. */
function meaningChanges(habit: PreviewHabit, draft: HabitDraft): boolean {
  return (
    habit.detail.period !== draft.period ||
    habit.detail.direction !== draft.direction ||
    habit.detail.target !== draft.target ||
    habit.detail.scheduleMask !== normalisedMask(draft.scheduleMask)
  );
}

/** How many marks sit on days a schedule does not include. */
function marksOutside(habit: PreviewHabit, mask: number): number {
  let outside = 0;
  for (const day of habit.marks.keys()) {
    if (!isScheduled(mask, day)) {
      outside += 1;
    }
  }
  return outside;
}

/**
 * The longest run the marks hold, whether or not it is the one running now.
 *
 * Walks the whole seeded window once rather than from every day, which matters only because
 * the detail screen asks for it on every paint and a browser tab has no other work to do.
 */
function longestStreakOf(habit: PreviewHabit): number {
  let longest = 0;
  let running = 0;
  for (let step = SEEDED_DAYS; step >= 0; step -= 1) {
    const day = dayPlus(today(), -step);
    if (day < habit.detail.startedOn || !isScheduled(habit.detail.scheduleMask, day)) {
      continue;
    }
    running = judge(habit, day).state === 'done' ? running + 1 : 0;
    longest = Math.max(longest, running);
  }
  return longest;
}

/**
 * The stand-in boundary.
 *
 * Annotated with the same shared type as the real one, so the two cannot drift: a command
 * added on one side and not the other stops the build rather than waiting to be noticed.
 */
export const ipc: IpcSurface = {
  previewNotice: `Previsualización de la interfaz. Todos los datos son inventados, no hay núcleo detrás y aquí no se cifra nada. ${MARKER}`,

  fetchInstanceStatus: () => Promise.resolve({ state: previewInstanceState() }),

  fetchAppInfo: () => Promise.resolve(PREVIEW_APP_INFO),

  fetchDiagnostics: () => Promise.resolve(PREVIEW_DIAGNOSTICS),

  insertSampleHabit: () => {
    previewHabitCount += 1;
    const habit: SampleHabit = {
      id: `00000000-0000-4000-8000-${String(previewHabitCount).padStart(12, '0')}`,
      name: SAMPLE_NAME,
      deleted: false,
      cursor: String(previewHabitCount).padStart(32, '0'),
    };
    previewHabits.push(habit);
    return Promise.resolve(habit);
  },

  listSampleHabits: (page) =>
    Promise.resolve(previewHabits.filter((habit) => !habit.deleted).slice(0, page.limit)),

  deleteSampleHabit: (id) => {
    const found = previewHabits.find((habit) => habit.id === id && !habit.deleted);
    if (found === undefined) {
      return reject({ kind: 'notFound' });
    }
    const gone: SampleHabit = { ...found, deleted: true };
    previewHabits.splice(previewHabits.indexOf(found), 1, gone);
    return Promise.resolve(gone);
  },

  /*
   * The one answer here that is worked out rather than fixed, and it has to be.
   *
   * A sweep is the only control on that screen whose whole point is what it removes, so a
   * stand-in that answered a number without removing anything would show a screen that cannot be
   * told apart from one over a core that silently did nothing. It sweeps both lists, because the
   * core has one table and this file has two: the diagnostics one, where a swept row becomes a
   * tombstone, and the habits one, where it simply stops being there.
   */
  sweepSampleHabits: () => {
    const started = Date.now();
    let removed = 0;

    for (const [at, habit] of previewHabits.entries()) {
      if (!habit.deleted && isSampleName(habit.name)) {
        previewHabits.splice(at, 1, { ...habit, deleted: true });
        removed += 1;
      }
    }

    for (let at = habits.length - 1; at >= 0; at -= 1) {
      const habit = habits[at];
      if (habit !== undefined && isSampleName(habit.detail.name)) {
        habits.splice(at, 1);
        removed += 1;
      }
    }

    return Promise.resolve({
      removed,
      remaining: habits.length,
      elapsedMs: Math.max(1, Date.now() - started),
    });
  },

  seedData: (rowsPerTable) =>
    Promise.resolve({ tables: [{ table: 'habits', rows: rowsPerTable }], elapsedMs: 42 }),

  // Fixed, like every other answer here. A stand-in that removed rows would be pretending to
  // have a retention window, and there is no file to have one about.
  compactTombstones: () => Promise.resolve({ tables: [], removed: 0, remaining: 0, elapsedMs: 7 }),

  fetchVaultStatus: () => Promise.resolve(status()),

  createVault: (password, params) => {
    const problem = validate(password);
    if (problem !== null) {
      return reject(problem);
    }
    if (vault !== null) {
      return reject({ kind: 'alreadyExists' });
    }

    vault = {
      password,
      unlocked: true,
      failedAttempts: 0,
      lockedUntilMs: 0,
      lastActivityMs: Date.now(),
      inactivity: 'five',
      kdf: report(params),
      condition: 'readable',
    };

    return Promise.resolve(status());
  },

  unlockVault: (password) => {
    if (vault === null) {
      return reject({ kind: 'noVault' });
    }

    const waiting = remainingSeconds(vault.lockedUntilMs);
    if (waiting > 0) {
      return reject({ kind: 'lockedOut', remainingS: waiting });
    }

    // Compared with an ordinary equality, because there is nothing here to derive a key
    // from. See the warning at the top of this file.
    if (password !== vault.password) {
      vault.failedAttempts += 1;
      vault.lockedUntilMs = Date.now() + backoffSeconds(vault.failedAttempts) * 1000;
      return reject({ kind: 'notOpened' });
    }

    vault.unlocked = true;
    vault.failedAttempts = 0;
    vault.lockedUntilMs = 0;
    vault.lastActivityMs = Date.now();

    return Promise.resolve(status());
  },

  lockVault: () => {
    if (vault !== null && vault.unlocked) {
      vault.unlocked = false;
      announceLock('requested');
    }
    return Promise.resolve(status());
  },

  changeMasterPassword: (current, next) => {
    if (vault === null) {
      return reject({ kind: 'noVault' });
    }
    const problem = validate(next);
    if (problem !== null) {
      return reject(problem);
    }
    if (current !== vault.password) {
      return reject({ kind: 'notOpened' });
    }

    vault.password = next;
    vault.unlocked = true;
    vault.lastActivityMs = Date.now();

    return Promise.resolve(status());
  },

  changeKdfParams: (password, params) => {
    if (vault === null) {
      return reject({ kind: 'noVault' });
    }
    if (password !== vault.password) {
      return reject({ kind: 'notOpened' });
    }

    vault.kdf = report(params);
    vault.unlocked = true;
    vault.lastActivityMs = Date.now();

    return Promise.resolve(status());
  },

  sendHeartbeat: () => {
    if (vault === null || !vault.unlocked) {
      return reject({ kind: 'locked' });
    }

    vault.lastActivityMs = Date.now();

    return Promise.resolve(status());
  },

  setInactivity: (choice) => {
    if (vault !== null) {
      vault.inactivity = choice;
      vault.lastActivityMs = Date.now();
    }
    return Promise.resolve(status());
  },

  estimatePasswordStrength: (password) => {
    // A coarser guess than the core's, and deliberately so: this exists to make the bar
    // move while somebody looks at the screen, not to advise anyone about a password.
    const chars = [...password].length;
    const kinds = [/\p{Ll}/u, /\p{Lu}/u, /\p{Nd}/u, /[^\p{L}\p{Nd}]/u].filter((pattern) =>
      pattern.test(password),
    ).length;
    const score = chars + kinds * 4;

    let strength: PasswordStrength = 'weak';
    if (score >= 34) {
      strength = 'strong';
    } else if (score >= 26) {
      strength = 'good';
    } else if (score >= 18) {
      strength = 'fair';
    }

    return Promise.resolve(strength);
  },

  /*
   * Exporting and verifying, which in a browser tab can only ever be refused.
   *
   * There is no file dialog to open, no disk to write to and no Argon2id to run, and a
   * stand-in that answered with an invented report would be a stand-in claiming a backup
   * exists. So both answer the way the real core answers somebody who closed the dialog:
   * nothing was chosen, so nothing happened. That is the one outcome of these two that a
   * browser can honestly reproduce, and it lets the screen around them be looked at with
   * every path through it intact.
   */
  exportBackup: () => rejectBackup({ kind: 'cancelled' }),

  verifyBackup: () => rejectBackup({ kind: 'cancelled' }),

  beginImport: () => rejectBackup({ kind: 'cancelled' }),

  exportPlaintext: () => rejectBackup({ kind: 'cancelled' }),

  /*
   * The two halves of a restore that follow a preparation, and neither can ever be reached
   * here, because `beginImport` above never hands out a token. They answer the way the core
   * answers a word it is not holding, which is the only honest thing a stand-in with no
   * staging database can say.
   */
  commitImport: () => rejectBackup({ kind: 'unknownToken' }),

  cancelImport: () => rejectBackup({ kind: 'unknownToken' }),

  /*
   * The one backup answer a browser tab can give truthfully. There is no vault and therefore
   * no backup of one, which is exactly what "never" means, and the reminder that follows from
   * it is the state the screen most needs to be looked at in.
   */
  backupStatus: () => Promise.resolve({ daysSinceLast: null, remind: true }),

  // Nothing ever runs here, so nothing ever reports progress. The handler is kept and
  // dropped so that a screen which subscribes and unsubscribes behaves as it will.
  onBackupProgress: (handler) => {
    progressListeners.add(handler);
    return Promise.resolve(() => {
      progressListeners.delete(handler);
    });
  },

  onVaultLocked: (handler) => {
    lockListeners.add(handler);
    return Promise.resolve(() => {
      lockListeners.delete(handler);
    });
  },

  /*
   * The four window controls.
   *
   * A browser tab is not a window this application owns: it cannot be dragged by its
   * content, it cannot be minimised, and closing it is not something a page may do
   * unasked. So these accept and do nothing, which is the honest behaviour — the buttons
   * are drawn and can be reached with the keyboard, and what they do belongs to a real
   * window.
   *
   * Only the maximise state is remembered, so that the button draws the right glyph and
   * somebody looking at the header can see both of them.
   */
  startWindowDrag: () => Promise.resolve(),

  minimizeWindow: () => Promise.resolve(),

  toggleMaximizeWindow: () => {
    maximised = !maximised;
    return Promise.resolve(maximised);
  },

  closeWindow: () => {
    // The vault half is real even here, and it is the half that matters: this is the one
    // place where a screen could be written against a close that left the vault open.
    if (vault !== null && vault.unlocked) {
      vault.unlocked = false;
      announceLock('requested');
    }
    return Promise.resolve();
  },

  /*
   * The eleven habits commands.
   *
   * These are the ones that change their own state rather than answering a constant, because
   * a list of habits that cannot be marked, reordered or emptied is a picture of a screen and
   * not a screen. Everything they answer is worked out by the judge above, from the marks in
   * this file, by rules that belong to this file alone.
   */

  listHabits: (filter) =>
    Promise.resolve(
      habits
        .filter((habit) => habit.archived === (filter === 'archived'))
        .sort((left, right) => left.position - right.position)
        .map(summaryOf),
    ),

  getHabit: (id) => {
    const habit = findHabit(id);
    return habit === undefined
      ? rejectHabits({ kind: 'notFound' })
      : Promise.resolve(detailOf(habit));
  },

  createHabit: (draft) => {
    const problems = problemsWith(draft);
    if (problems.length > 0) {
      return rejectHabits({ kind: 'invalid', problems });
    }
    inventedHabits += 1;
    const created = makeHabit(
      {
        id: `00000000-0000-4000-8000-b${String(inventedHabits).padStart(11, '0')}`,
        name: draft.name,
        icon: draft.icon,
        color: draft.color,
        period: draft.period,
        unit: draft.unit,
        target: draft.target,
        direction: draft.direction,
        scheduleMask: normalisedMask(draft.scheduleMask),
        notes: draft.notes,
        startedOn: draft.startedOn,
        aggregation: draft.aggregation,
      },
      habits.length,
    );
    habits.push(created);
    return Promise.resolve(detailOf(created));
  },

  updateHabit: (id, draft) => {
    const habit = findHabit(id);
    if (habit === undefined) {
      return rejectHabits({ kind: 'notFound' });
    }
    const problems = problemsWith(draft);
    if (problems.length > 0) {
      return rejectHabits({ kind: 'invalid', problems });
    }
    const streakMeaningChanged = meaningChanges(habit, draft);
    applyDraft(habit, draft);
    return Promise.resolve({ habit: detailOf(habit), streakMeaningChanged });
  },

  previewHabitUpdate: (id, draft) => {
    const habit = findHabit(id);
    if (habit === undefined) {
      return rejectHabits({ kind: 'notFound' });
    }
    const problems = problemsWith(draft);
    if (problems.length > 0) {
      return rejectHabits({ kind: 'invalid', problems });
    }
    // Worked out on a copy, so that asking what would happen never makes it happen. The
    // marks are shared on purpose: the question is what the same calendar becomes.
    const after: PreviewHabit = { ...habit, detail: { ...habit.detail } };
    const currentStreakBefore = streakOf(habit);
    applyDraft(after, draft);
    return Promise.resolve({
      streakMeaningChanged: meaningChanges(habit, draft),
      currentStreakBefore,
      currentStreakAfter: streakOf(after),
      entriesOutsideNewSchedule: marksOutside(habit, normalisedMask(draft.scheduleMask)),
    });
  },

  archiveHabit: (id, archived) => {
    const habit = findHabit(id);
    if (habit === undefined) {
      return rejectHabits({ kind: 'notFound' });
    }
    habit.archived = archived;
    return Promise.resolve(summaryOf(habit));
  },

  deleteHabit: (id) => {
    const at = habits.findIndex((habit) => habit.detail.id === id);
    if (at < 0) {
      return rejectHabits({ kind: 'notFound' });
    }
    habits.splice(at, 1);
    return Promise.resolve();
  },

  reorderHabits: (ids) => {
    // The whole set or nothing, exactly as the core insists, because a screen written against
    // a stand-in that accepted half an order would meet the refusal for the first time in the
    // window.
    const live = habits.filter((habit) => !habit.archived);
    const wanted = new Set(ids);
    if (wanted.size !== ids.length || live.some((habit) => !wanted.has(habit.detail.id))) {
      return rejectHabits({ kind: 'incompleteOrder' });
    }
    ids.forEach((id, position) => {
      const habit = findHabit(id);
      if (habit !== undefined) {
        habit.position = position;
      }
    });
    return Promise.resolve();
  },

  toggleHabitDay: (id, day, amount) => {
    const habit = findHabit(id);
    if (habit === undefined) {
      return rejectHabits({ kind: 'notFound' });
    }
    if (day > today()) {
      return rejectHabits({ kind: 'dayInFuture' });
    }
    if (day < habit.detail.startedOn) {
      return rejectHabits({ kind: 'dayTooOld' });
    }
    // A habit that counts a quantity is set to the amount it is given, and nothing means the
    // mark goes away. A habit that is simply done or not is a switch, and what the switch
    // records is one of the thing: one session done, or one slip had.
    if (!countsQuantity(habit)) {
      if (habit.marks.has(day)) {
        habit.marks.delete(day);
      } else {
        habit.marks.set(day, 1);
      }
    } else if (amount === null) {
      habit.marks.delete(day);
    } else {
      habit.marks.set(day, amount);
    }
    return Promise.resolve(judge(habit, day));
  },

  habitHeatmap: (id, year) => {
    const habit = findHabit(id);
    if (habit === undefined) {
      return rejectHabits({ kind: 'notFound' });
    }
    const years = [...habit.marks.keys()].map((day) => Math.trunc(day / 10_000));
    return Promise.resolve({
      year,
      days: daysOfYear(year).map((day) => ({ day, state: judge(habit, day) })),
      firstYearWithData: years.length === 0 ? null : Math.min(...years),
    });
  },

  habitStats: (id) => {
    const habit = findHabit(id);
    if (habit === undefined) {
      return rejectHabits({ kind: 'notFound' });
    }
    const marked = [...habit.marks.keys()].sort((left, right) => left - right);
    const now = today();
    const monthStart = Math.trunc(now / 100) * 100 + 1;
    let done = 0;
    let of = 0;
    // Today is not counted. The month's percentage is about days that are over, and a day
    // still open would otherwise be a miss for as long as nobody had got round to it.
    for (let day = monthStart; day < now; day = dayPlus(day, 1)) {
      const state = judge(habit, day).state;
      if (state === 'done' || state === 'missed') {
        of += 1;
        if (state === 'done') {
          done += 1;
        }
      }
    }
    return Promise.resolve({
      current: streakDtoOf(habit),
      longest: longestStreakOf(habit),
      monthCompletion: { done, of, percent: of === 0 ? 0 : Math.round((done * 100) / of) },
      totalEntries: marked.length,
      firstDay: marked[0] ?? null,
      lastDay: marked[marked.length - 1] ?? null,
    });
  },
};
