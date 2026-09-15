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

| Command                | What it catches                                     |
| ---------------------- | --------------------------------------------------- |
| `npm run format:check` | Formatting drift.                                   |
| `npm run lint`         | Lints, including the rules that defend the WebView. |
| `npm run check`        | Types, using the Svelte compiler.                   |
| `npm run knip`         | Files, exports and dependencies nothing uses.       |
| `npm audit --omit=dev` | Known vulnerabilities in anything that would ship.  |

Four lint rules exist for the threat model rather than for tidiness, and a change that trips one of them is not a style disagreement.

`svelte/no-at-html-tags` forbids `{@html}` anywhere. A vault entry imported from a file somebody else wrote is hostile input. Rendering it as HTML turns a malicious backup into script running inside the WebView, and script inside the WebView is one step away from the command boundary. There is no legitimate use for it in this project.

`no-restricted-imports` stops any file except `src/lib/ipc.ts` from importing the Tauri API. The value of one narrow, auditable boundary is lost entirely if a component can reach past it, and that is not something review reliably notices.

`no-restricted-syntax` and `no-restricted-globals` ban `eval`, the `Function` constructor, and assignment to `innerHTML` or `outerHTML`, which are the remaining ways to execute code or inject markup at runtime.

Each of these was verified by writing a deliberate violation and confirming the linter rejects it, rather than by confirming the rule is present in a configuration file.

### Why type-aware linting is off inside components

This is a division of labour, not a hole.

The lint parser compiles a component into virtual TypeScript in order to type it, and that transformation does not carry the narrowing a template performs. Code inside `{#if status.kind === 'ready'}` reads as `any` to the type-aware rules, so they report a union that is in fact fully discriminated. Leaving them on would produce either a wall of false positives or a scattering of suppressions, and a suppression people learn to add without reading is worse than no rule at all.

Types inside components are checked by `npm run check`, which runs the Svelte compiler itself and understands the narrowing. That gate is not optional, and it is the one that catches a real type error in a component. The rules that defend the WebView are syntactic and apply everywhere regardless.

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

## The two tests that read configuration

`src-tauri/tests/config_hardening.rs` parses `tauri.conf.json` and asserts every setting the WebView defence depends on: the isolation pattern, a content security policy with no inline or evaluated script, no global Tauri object, prototypes frozen, the asset protocol disabled, drag and drop off, and no capability granting more than the core defaults.

It is the most valuable test in the project so far, because of the way the failure it guards against behaves. Loosening the content security policy breaks nothing. The application starts, every screen works, every other test passes, and the protection is simply gone. Nobody notices until it matters. It was verified by loosening each setting in turn and confirming the test fails for each one.

`src-tauri/tests/diagnostics_privacy.rs` asserts that the diagnostics snapshot contains no drive letter, no home directory, and no value of any environment variable naming the account or the machine. It reads those from the environment rather than a hard coded name, so it is meaningful on whatever machine it runs on, and it fails rather than passing if none of them was set, because a check that verified nothing must not report success.

## Advisories that are currently open

`cargo audit` reports no vulnerabilities and seven warnings. All seven are transitive dependencies of the application framework: six unmaintained crates reached through its macro and text handling, and one unsoundness in a GTK binding that only compiles on Linux, which is not a target platform for the application.

None of them is something this project can fix by changing its own code, and none is reachable from a code path here. They are listed rather than silenced, so that the next person to run the audit recognises them instead of assuming somebody already decided they were fine.

## Performance budgets

These are measured, not estimated, and a budget without a measured number behind it is not treated as met.

| Measure                    | Budget | Measured                                 |
| -------------------------- | ------ | ---------------------------------------- |
| Release executable size    | 15 MB  | 4.0 MB                                   |
| Command round trip         | 5 ms   | 1.5 to 1.9 ms, median of nine warm calls |
| Cold start to first answer | 400 ms | 469 to 475 ms, over budget               |

Cold start is the honest number and it does not meet its budget. It is measured from the uptime the core reports at the moment the interface receives its first answer, so it covers the whole wait: process, window, WebView, bundle and one round trip. Most of it is the WebView initialising, which is work this project does not control and has not yet tried to overlap with anything useful. It is recorded as over rather than quietly rounded down, and it is worth revisiting when there is enough of an application for the difference to be noticeable.

The size and latency figures are comfortable, and both will get worse as the application grows. Having the baseline now is the point: it turns a future argument about whether things used to feel faster into a comparison between two numbers.
