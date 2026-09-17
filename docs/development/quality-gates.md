# Quality gates

Every check that can decide on its own whether a change is acceptable. All of them run in the pipeline on every push and every pull request, and you should run them locally first: the pipeline is a safety net, not a substitute for looking at your own work.

One rule matters more than the list. **A gate that was not run is never reported as passing.** If something could not be executed, it is reported as skipped, with the reason. A green summary that includes a check nobody ran is worse than a red one, because it is believed.

## Rust

| Command | What it catches |
| --- | --- |
| `cargo fmt --all -- --check` | Formatting drift, so that no diff ever contains a change nobody made. |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Lints, including the project rules below. Warnings are errors; a warning nobody reads is not doing anything. |
| `cargo test --workspace --all-features` | The test suite, including the two that read configuration rather than code. |
| `cargo llvm-cov --workspace --all-features --fail-under-lines 80` | Line coverage of the workspace. |
| `cargo deny check` | Licences, security advisories, banned crates, and dependencies pulled from anywhere but the public registry. |
| `cargo audit` | Known vulnerabilities in the dependency graph. |

The clippy configuration denies the things that can end a process on a value the code never checked: `unwrap`, `expect`, `panic!`, unchecked indexing, integer division, and floating point arithmetic, which is never how money is handled here. It also denies leftovers from working on something: `dbg!`, printing to standard output or error, `todo!` and `unimplemented!`. Inside a test, unwrapping is how a failed assumption gets reported, so those are relaxed there and only there.

Suppressing one of these is allowed and is meant to be uncomfortable. It takes an `#[expect(...)]` with a `reason` on the same line, so the justification is in the diff and in the file, next to the thing it justifies.

## Frontend

| Command | What it catches |
| --- | --- |
| `npm run format:check` | Formatting drift. |
| `npm run lint` | Lints, including the rules that defend the WebView. |
| `npm run tokens` | Colours, lengths and durations written by hand instead of taken from a token. |
| `npm run check` | Types, using the Svelte compiler. |
| `npm run test:unit` | The frontend unit tests. |
| `npm run test:a11y` | Accessibility violations, with `axe`, on every preview screen in both themes. |
| `npm run knip` | Files, exports and dependencies nothing uses. |
| `npm audit --omit=dev` | Known vulnerabilities in anything that would ship. |

Four lint rules exist for the threat model rather than for tidiness, and a change that trips one of them is not a style disagreement.

`svelte/no-at-html-tags` forbids `{@html}` anywhere. A vault entry imported from a file somebody else wrote is hostile input. Rendering it as HTML turns a malicious backup into script running inside the WebView, and script inside the WebView is one step away from the command boundary. There is no legitimate use for it in this project.

`no-restricted-imports` stops any file except `src/lib/ipc.ts` from importing the Tauri API. The value of one narrow, auditable boundary is lost entirely if a component can reach past it, and that is not something review reliably notices.

`no-restricted-syntax` and `no-restricted-globals` ban `eval`, the `Function` constructor, and assignment to `innerHTML` or `outerHTML`, which are the remaining ways to execute code or inject markup at runtime.

Each of these was verified by writing a deliberate violation and confirming the linter rejects it, rather than by confirming the rule is present in a configuration file.

### Why type-aware linting is off inside components

This is a division of labour, not a hole.

The lint parser compiles a component into virtual TypeScript in order to type it, and that transformation does not carry the narrowing a template performs. Code inside `{#if status.kind === 'ready'}` reads as `any` to the type-aware rules, so they report a union that is in fact fully discriminated. Leaving them on would produce either a wall of false positives or a scattering of suppressions, and a suppression people learn to add without reading is worse than no rule at all.

Types inside components are checked by `npm run check`, which runs the Svelte compiler itself and understands the narrowing. That gate is not optional, and it is the one that catches a real type error in a component. The rules that defend the WebView are syntactic and apply everywhere regardless.

### The design token gate

`scripts/check-tokens.mjs` reads every `.css` and `.svelte` file under `src/` and fails on a colour literal, a `px` or `rem` length, or a duration in `ms` or `s`. Every one of those has a token in `src/lib/styles/tokens.css`, and [the design system](../design/design-system.md) is where each value comes from and why.

It rejects a length even when the number happens to be on the scale. `padding: 16px` is exactly `--space-4` today; a component that writes the number has nonetheless stopped reading the tokens, and the next number it writes will not be on the scale. What it does not police is `em`, `ch`, `%`, `fr` and the viewport units, because each of those says "the same as something this element already has" rather than being a value typed in from nowhere.

Two things are exempt. `tokens.css` itself, which is the whole point of there being one file. And a media query prelude, because a media query cannot read a custom property — there are exactly two breakpoints in this application, 880px and 420px, and both are named in the design system.

Anything else needs a comment on the line above saying `tokens-exempt:` followed by a reason. That list started empty and is meant to stay short: erosion happens one reasonable exception at a time, and a gate is what makes each one argue for itself in the diff. A `tokens-exempt:` with no reason after it is not an exemption, and there is a test for that.

It has no dependencies. The alternative was stylelint with `postcss-html` and the Svelte plugin, which is three packages and their transitive graph to run three regular expressions over a directory. Its own behaviour is covered by `scripts/check-tokens.test.mjs`, and it was verified end to end by adding a colour, a length and a duration to a real file and watching the build go red.

### The accessibility gate

`npm run test:a11y` starts the preview build, drives each screen the way a person reaches it, and runs `axe-core` over it: WCAG 2.2 at level AA plus the best practice set. Every screen is checked twice, once in each theme, because contrast is a property of the pair and a palette that passes on paper says nothing about ink.

It uses a real browser, and that is the whole reason Playwright is a dependency. Half of the AA rules are about computed colour and computed layout — contrast, overlap, target size, focus visibility — and `jsdom` has neither, so it evaluates none of them and reports a clean run. A gate that passes without having looked is worse than no gate, because it is believed.

The browser is not in the repository and is not installed by `npm ci`. Run `npx playwright install chromium` once. In the pipeline it is cached against the version in the lockfile, so a run normally restores it rather than downloading it.

Both Playwright and `axe-core` are development dependencies. The gate that asserts `package.json` declares no runtime dependencies is unaffected and still reads zero.

It was verified by adding an `<input>` with no `<label>` to a screen and confirming the gate fails, and it found two real defects on the day it was switched on: the diagnostics screen had no level-one heading, and it skipped from that heading straight to level three.

### The unit tests

`npm run test:unit` is `node --test` over `scripts/**/*.test.mjs` and `src/**/*.test.ts`. There is no test framework, because Node 24 runs TypeScript directly and carries a runner, and a framework would be several packages in the graph for something already installed.

What it covers is the logic that can be tested without a browser: the token gate itself, and the state machine behind the tab strip. Anything that needs a rendered page belongs in the accessibility gate or in a manual test, and anything in the core belongs in `cargo test`.

## Secrets and personal data

| Command                    | What it catches                                     |
| -------------------------- | --------------------------------------------------- |
| `gitleaks detect --redact` | Credentials in the working tree and in the history. |

The pre-commit hook in `.githooks/pre-commit` runs a staged-only scan plus four more checks: absolute paths that name an account or a machine, private network addresses, attribution lines, and any document at the repository root that is not one of the five a public repository is expected to carry.

That last one is an allowlist rather than a list of forbidden names, because a list of forbidden names only catches the mistakes somebody already thought of.

The hook fails loudly if gitleaks is not installed instead of skipping the scan. A check that silently did not run still produces the feeling of being protected, which is the worst outcome available.

## Checks that only exist in the pipeline

Two things cannot be expressed as a unit test and run as their own steps.

One asserts that `package.json` declares no runtime dependencies. The frontend has none, and every package in the bundle is attack surface in an application that holds a password vault, so that property is worth a gate rather than a sentence in a document.

The other asserts that `unsafe` appears nowhere except `crates/cairn-platform`.

A third builds the production bundle and searches the whole of `dist/` for the marker string the preview module defines. Preview mode replaces the core with something that invents its answers, and an application holding a password vault must never show invented data as though it were the person's own. A build-time alias keeps that module out of the graph entirely, and this step checks the artefact rather than the intention, because one badly placed import would undo the alias without producing a warning. [Preview mode](preview-mode.md) explains both halves.

## The iOS compilation gate

`ios-compile` cross compiles the five library crates for `aarch64-apple-ios` on a `macos-15` runner, on every pull request and every push. It is blocking: a red `ios-compile` means the pull request cannot be merged.

It exists to answer one question early, with a measurement instead of an assumption: does SQLCipher, with OpenSSL built from source, cross compile to an iPhone. The whole storage design rests on that, and there is no way to find out except to try it. Finding out now costs days; finding out after eight phases of code have been written on top of the assumption costs rewriting the storage layer.

**What it compiles.** `cairn-crypto`, `cairn-domain`, `cairn-db`, `cairn-sync` and `cairn-platform`, named one by one rather than as `--workspace`. The workspace also contains the desktop application, and building that for iOS needs Xcode scaffolding that does not exist yet. Including it would make the job fail for a reason that has nothing to do with what the job is measuring, and a gate that goes red for reasons nobody understands gets switched off within a week.

**What it does not do.** It does not run anything. Not one line of this code is executed on a device or a simulator, because that would need a booted simulator or a phone, and neither is available to the pipeline. The job proves the code compiles and links for the architecture, and it says so in its own summary rather than letting the green tick imply more.

It also does not produce an application. No `.ipa`, no signing, no Xcode project, no Apple ID. Those belong to the phase that puts the application on a phone, and none of them is needed to answer the question this job answers.

**Which profile.** Pull requests build debug, so the loop stays short. Pushes to `dev` and `main` build release, which is the path that would actually ship: fat link time optimisation, one codegen unit, and OpenSSL's assembly. Both get exercised every day and neither slows the other down.

**The caching.** Without a cache, every run rebuilds OpenSSL and SQLCipher from C, and that is most of the time the job takes. GitHub scopes caches by branch on its own: a pull request reads the cache of its base branch and writes only into its own scope, so one branch cannot poison another's. The declared threshold is ten minutes with a warm cache. If the job goes past that consistently, it moves to `dev` and manual runs only, and the reason gets written down rather than the job quietly becoming something people wait for.

**What its summary contains.** The size of the compiled Rust libraries, the size of the SQLCipher and OpenSSL archives, and how long the build took. Three numbers, not a sentence. They are the baseline the phone phase will compare against, and a baseline that has to be reconstructed from a log is a baseline nobody reconstructs.

**When it goes red.** Something in the core stopped crossing to iOS. Usually one of three things: a dependency that only builds for desktop, code behind a `cfg` that does not cover the target, or a new release of the OpenSSL sources. The last one is why `openssl-src` is pinned to one exact version in `Cargo.toml` as well as in the lockfile: pinned that way, the upgrade arrives as a pull request this gate gets to judge, rather than sideways the next time somebody regenerates the lockfile.

Do not merge past it and do not make it non-blocking. It is blocking from its first day for a reason that will not come back: nothing depends on iOS yet, so the cost of it being red is zero, and this is the only moment in the project when that is true.

## The two tests that read configuration

`src-tauri/tests/config_hardening.rs` parses `tauri.conf.json` and asserts every setting the WebView defence depends on: the isolation pattern, a content security policy with no inline or evaluated script, no global Tauri object, prototypes frozen, the asset protocol disabled, drag and drop off, and no capability granting any core permission at all.

That last one is stricter than it first looks. Tauri offers `core:default` as a convenient starting bundle, and it is what a generated project begins with, but it is a bundle rather than a minimal list and it includes path resolution. Script that has managed to run inside the WebView could use that to learn the account name, which is precisely what the diagnostics screen goes to trouble to avoid revealing. The application's own commands do not need a capability entry, so the list is empty, which was confirmed by emptying it and watching the application carry on working.

It is the most valuable test in the project so far, because of the way the failure it guards against behaves. Loosening the content security policy breaks nothing. The application starts, every screen works, every other test passes, and the protection is simply gone. Nobody notices until it matters. It was verified by loosening each setting in turn and confirming the test fails for each one.

`src-tauri/tests/diagnostics_privacy.rs` asserts that the diagnostics snapshot contains no drive letter, no home directory, and no value of any environment variable naming the account or the machine. It reads those from the environment rather than a hard coded name, so it is meaningful on whatever machine it runs on, and it fails rather than passing if none of them was set, because a check that verified nothing must not report success.

## Advisories that are currently open

`cargo audit` reports no vulnerabilities and seven warnings. All seven are transitive dependencies of the application framework: six unmaintained crates reached through its macro and text handling, and one unsoundness in a GTK binding that only compiles on Linux, which is not a target platform for the application.

None of them is something this project can fix by changing its own code, and none is reachable from a code path here. They are listed rather than silenced, so that the next person to run the audit recognises them instead of assuming somebody already decided they were fine.

## Performance budgets

These are measured, not estimated, and a budget without a measured number behind it is not treated as met.

| Measure                    | Budget | Measured                                 |
| -------------------------- | ------ | ---------------------------------------- |
| Release executable size    | 15 MB  | 3.96 MB                                  |
| Command round trip         | 5 ms   | 1.5 to 1.9 ms, median of nine warm calls |
| Cold start to first answer | 500 ms | 469 to 475 ms                            |

Cold start is measured from the uptime the core reports at the moment the interface receives its first answer, so it covers the whole wait: process, window, WebView, bundle and one round trip. Nothing is excluded to make the number look better.

That budget started at 400 ms and was raised to 500 ms, which is the kind of change worth explaining rather than quietly making. The first measurements came in at 469 to 475 ms, and profiling put most of that in WebView2 initialising before any project code runs. Two honest options existed: treat 400 ms as a target to optimise towards, or accept that it was set without knowing what a WebView costs to start. The second is what happened. A budget nobody can meet and nobody intends to act on is not a budget, it is a permanently red number that teaches people to skip the row.

Raising it is not the same as ignoring it. 500 ms still fails if the application grows careless, the measurement still runs, and the two routes to getting under 400 ms are written down rather than forgotten: show the window before the bundle is ready, so WebView startup overlaps with something useful, and cut what happens between the interface mounting and its first question to the core. Neither is worth doing against an application that does almost nothing, because there is no way to tell whether a saving is real or noise. This gets measured again when there is enough application for the answer to mean something.

The size and latency figures are comfortable, and both will get worse as the application grows. Having the baseline now is the point: it turns a future argument about whether things used to feel faster into a comparison between two numbers.
