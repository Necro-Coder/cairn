# Project layout

What lives where, and more importantly what is allowed to depend on what.

## The shape of it

```
crates/
  cairn-crypto/      key derivation, the key hierarchy, authenticated encryption, export framing
  cairn-domain/      business logic: streaks, budgets, validation, merge rules
  cairn-db/          schema, migrations, repositories, opening the encrypted database
  cairn-sync/        framing, the pre-shared-key handshake, the merge protocol
  cairn-platform/    the one crate allowed to contain unsafe code
src-tauri/
  src/commands/      the boundary between the WebView and the core
  src/state.rs       state that lives as long as the window
  capabilities/      the access control list the runtime enforces
  tauri.conf.json    window, content security policy, isolation pattern
  tests/             the tests that read configuration rather than code
src-isolation/       the sandboxed frame every message passes through
src/
  lib/ipc.ts         the only file that may import the Tauri API
  lib/styles/        design tokens and element defaults
  lib/fonts/         the two bundled typefaces, as assets, with their licences
  lib/icons/         the eighteen hand-written icons and the brand glyph
  lib/shell/         the header, the tab strip, the panel and the command palette
  lib/search/        the search contract every module registers a provider against
  routes/            screens
  routes/modules/    habits, passwords and finances, drawn before they work
  routes/settings/   the five parts of the settings screen, diagnostics among them
scripts/             the design token gate, and its tests
tests/a11y/          the accessibility gate
assets/icon.svg      the source the application icons are generated from
docs/                this documentation
.githooks/           the pre-commit hook, kept in the repository so it can be audited
```

## The frontend, in more detail

`lib/styles/` is the design system as code. `tokens.css` holds every value in the interface and is the only file in `src/` allowed to contain one; `base.css` declares the two bundled faces and the element defaults everything inherits. [The design system](../design/design-system.md) is where those values come from, and `npm run tokens` is what stops a component from typing its own.

`lib/fonts/` holds two `woff2` files and the `OFL.txt` that has to ship beside each. They are assets, not packages: nothing in `package.json` refers to them, so the runtime dependency count stays at zero. The reasoning is in [decision 0006](../architecture/decisions/0006-tabbed-navigation-and-bundled-type.md).

`lib/icons/` holds eighteen icons and one brand glyph, all hand-written inline SVG. The box, the stroke weight and the decision about assistive technology live once in `IconFrame.svelte`; each icon file contributes its geometry and nothing else. The set is enumerated in the design system, and growing it means adding a line there saying what the new one is for.

`lib/shell/` is the application frame: the header, the tab strip, the panel and the command palette. Every rule in it is a pure module beside the component that draws it, so it is tested as ordinary functions over ordinary values rather than through a rendered page. `tabs.ts` holds the rules of the strip — born temporary, replace the temporary slot, pin on double click, cap at six and at four — and `palette/commands.ts` holds the matching, including the fold that makes `habitos` find `Hábitos`. `workspace.ts` holds the three things an open vault leaves behind: the tabs, the cards on the panel and what has been run from the palette.

That third one is the reason the workspace is one object. `workspace.svelte.ts` is the only reactive state the shell reads, and closing the vault sets it to null in a single assignment, which takes all three with it. Anything derived from an open vault that a later phase adds goes in there and is discarded by the line that already exists, rather than by a line somebody has to remember to write — which is exactly what was forgotten once, in PR #31, where a panel drawn above everything else survived a lock.

`lib/search/` is the contract every module will register a search provider against, written before there is anything to search. A hit carries a module, an identity, a title and a date, and there is no field for content, a value, an amount or a snippet: the palette opens on two keys, so the type is what makes showing a secret impossible rather than the care of whoever writes the provider. `contract.test-d.ts` fails to compile if a fifth field appears, whatever it is called. Both arguments a provider is handed are bounded before it sees them, at fifty hits and at two hundred characters of query, because in phase 03 that query stops being matched against eight titles in the WebView and becomes an argument to a command in the core.

`routes/` holds the screens. A screen knows nothing about tabs; it is what the shell draws inside whichever one is active. `routes/modules/` is habits, passwords and finances. Habits is real and is described below; the other two are still drawn in full before they work: the real empty state somebody meets on their first day, the shape the data will take — a list of accounts, three totals of a month — and a badge where the value will go saying the part is not written. Nothing in those two reads or writes anything, and the figures shown are zero because a vault with nothing in it holds nothing, which is a fact rather than a placeholder. `routes/settings/` is the five parts of the settings screen — security, appearance, data, diagnostics and shortcuts — with the diagnostics drawn twice: as one of those parts while the vault is open, and as a screen of its own while it is closed, because the machine somebody needs a version number on is usually the one that will not open.

## The shape of a product module

Habits is the first module with something behind it, and the shape it took is the shape passwords and finances are meant to copy. It is written down here because the second and third time it is built, the pressure will be to take a shortcut, and the shortcuts are all in the same two places: rules that end up in a screen, and a second opinion about the same fact on the two sides of the bridge.

Five layers, and what each one is allowed to decide.

| Where | What lives there | Habits |
| --- | --- | --- |
| `crates/cairn-domain/src/<module>/` | Every rule, as functions over values. No clock, no database, no input or output | `habits/{calendar,spec,day,streak,completion}.rs` |
| `crates/cairn-db/src/<module>/` | Rows in and out, and nothing that decides what a row means | Reads, writes, windows of days, the year a heat map covers |
| `src-tauri/src/commands/<module>.rs` | What today is, what the limits are, and the shape crossing the bridge | Zone, day-start preference, window sizes, the thirty-day limit, the DTOs |
| `src/routes/<module>/` | Screens, and pure modules beside them holding what a screen works out | `today.ts`, `detail.ts`, `draft.ts` beside their three components |
| `src/lib/ipc.types.ts` and `src/lib/ipc.ts` | The vocabulary, and the only file that may call the core | Eleven functions, one type per thing that crosses |

Four rules hold it together, and each answers a mistake that was made once.

**The domain takes today as an argument.** Every function in `habits/` that could need a date is handed one. Which day is today depends on a time zone and on the hour the person starts their day at, and the crate that knows the rules is not allowed to know either. It makes every rule testable without a clock, and it puts the one place that answers "what day is it" in the command layer where the preference and the zone already are.

**A verdict is computed once and travels.** `day::classify` returns one of five variants and everything downstream matches on those five. The interface receives conclusions — the streak, whether it is at risk, the percentage already rounded — and never the material to derive them. A screen that can derive a number is a screen that will eventually derive it differently from the core. The full reasoning is [decision 0012](../architecture/decisions/0012-what-a-streak-means.md).

**Both sides of the bridge are checked against one type.** `IpcSurface` in `ipc.types.ts` is implemented by `ipc.ts`, which calls Tauri, and by `lib/preview/ipc.ts`, which invents data for a browser. Neither can drift from the other, because a method added to one and not the other stops the typecheck. Before that type existed, the preview and the real bridge disagreed about an argument and nothing said so.

**A screen has four states and a union that makes the fifth impossible.** `lib/ui/async.ts` holds `Async<T>` — loading, empty, ready, failed — and `AsyncView.svelte` draws the first and the last so no screen invents its own. Every error variant the core can send has one Spanish sentence, written in one place, with the default branch typed as `never`: a new variant in Rust fails the build on this side rather than reaching somebody as a code.

Two smaller conventions that come with it. Logic a screen works out lives in a `.ts` file beside the component, not inside it, so `node --test` can reach it without a browser — the round trip a form makes through a habit and back is a test, not a hope. And no component contains a literal value: colours, sizes, spaces and durations come from `lib/styles/tokens.css`, and `npm run tokens` fails the build over a hand-written one.

## Dependencies point inwards

```
commands  →  domain  →  crypto
```

The domain does not know that SQLite exists, that Tauri exists, or what the wire format is. It is deterministic: the clock and the source of randomness are passed in by the caller rather than read from the environment, which is what lets a test pin both and get the same answer on every machine.

The cryptography crate goes further and performs no input or output at all. Everything it does is a function of its arguments. That is not purity for its own sake: it is what makes the crate checkable with property tests and under Miri, and what keeps the part of the system that must be right small enough to read in one sitting.

The command layer is a thin shell. It validates, delegates and maps errors. Logic worth testing does not live there, because testing it would mean starting a window.

## Why five crates, all created empty

They were created before there was anything to put in them, which looks like ceremony and is not. A boundary between modules is respected only if it already exists. Created later, at the moment something needs to cross it, it gets crossed for convenience and never uncrossed. The full reasoning is in [decision 0001](../architecture/decisions/0001-workspace-and-crate-boundaries.md).

## The unsafe rule

`unsafe` is denied across the whole workspace, and four of the five crates additionally carry `#![forbid(unsafe_code)]`, which cannot be turned off by anything inside the crate.

`cairn-platform` is the exception, and it is the reason the crate exists as a separate thing. Locking pages in memory, excluding a window from screen capture, and talking to the platform key stores all require calling raw platform APIs. That code has to live somewhere, and the useful decision is that it lives in one place that can be audited on its own rather than being scattered through whichever module happened to need it.

It is set to `deny` rather than `forbid` so that an individual call can opt out at the point of use with a `// SAFETY:` comment justifying every invariant it relies on. A blanket allow over the whole crate would defeat the point: the exception is argued once per call site, not once per crate.

A check in the pipeline asserts that no `unsafe` appears anywhere else, so the rule does not depend on anybody remembering it.

## The boundary with the WebView

Everything the interface can ask the core to do goes through a `#[tauri::command]`, and every call from the interface goes through `src/lib/ipc.ts`. Nothing else imports the Tauri API, and the linter enforces that rather than trusting review.

The value is that the entire attack surface between untrusted script and the core is one short file. A boundary nobody can enumerate is a boundary nobody can audit.

A command is a complete business operation, not a generic accessor. `unlock_vault` is a command; `get_field` is not, because it turns the boundary into an open query interface and invites calling it in a loop.

`src-isolation/` is a separate origin that Tauri loads in a sandboxed frame, and every message from the interface to the core passes through it. The hook there passes messages through unchanged today. Its value is not what it does but what it prevents: script that has managed to run in the application window cannot reach the core directly any more.

## Where the version comes from

One place: `[workspace.package]` in the root `Cargo.toml`. Every crate inherits it, the Tauri configuration derives from it rather than declaring its own, and a test asserts that what the core reports matches what the manifest says. Three copies of a version number are three places to forget.

## What is not here yet

`cairn-sync` is still a version number and a doc comment. Each crate arrives with the work that justifies it, so that it can be reviewed on its own rather than buried in a commit that also moves scaffolding around, and nothing synchronises yet.

Of the modules, only habits is real. Passwords and finances are screens with no core behind them, and they say so on themselves rather than leaving it to be discovered.
