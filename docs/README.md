# Cairn documentation

Three ways in, depending on what you want.

- **[Using Cairn](#using-cairn)** — you want to install it and use it.
- **[How it works](#how-it-works)** — you want to understand, audit or verify it.
- **[Working on Cairn](#working-on-cairn)** — you want to build it or contribute.

> The project is in early development. Pages marked _planned_ do not exist yet; they are written as the feature they describe is finished, so that nothing here describes something that is not real.

## Using Cairn

| Page                    | What it covers                                                        |
| ----------------------- | --------------------------------------------------------------------- |
| Installation on Windows | _planned_                                                             |
| Installation on iPhone  | _planned_                                                             |
| Getting started         | _planned_ — creating your master password and first records           |
| Security model          | _planned_ — what Cairn protects, and what it does not, in plain words |
| Backup and restore      | _planned_ — how encrypted exports work and why you need them          |
| Syncing your devices    | _planned_                                                             |
| Habits                  | _planned_                                                             |
| Vault                   | _planned_                                                             |
| Finances                | _planned_                                                             |
| Troubleshooting         | _planned_                                                             |

The user interface is in Spanish. This documentation is in English. See the note at the end of the [project README](../README.md).

## How it works

| Page | What it covers |
| --- | --- |
| Overview | _planned_ — the whole system in one page, read this first |
| Threat model | _planned_ — who the attacker is and what each defense answers |
| Cryptography | _planned_ — key hierarchy, AEAD, nonces, associated data |
| Data model | _planned_ — the schema and why every column exists |
| Storage | _planned_ — the two layers of encryption at rest |
| Backup format | _planned_ — the export file, byte by byte |
| Sync | _planned_ — transport, handshake and the merge algorithm |
| Platform hardening | _planned_ — operating system side channels on Windows and iOS |
| iOS pipeline | _planned_ — how the app is built and signed without a Mac |
| [Decision records](architecture/decisions/) | The choices that are expensive to reverse, including what was rejected and why. |

Two things this section will always do: explain the reasoning rather than restate the code, and be explicit about the weaknesses. A security document that only lists strengths is marketing.

## Working on Cairn

| Page | What it covers |
| --- | --- |
| [Getting started](development/getting-started.md) | From a fresh clone to a running app, and what to do when something does not work. |
| [Project layout](development/project-layout.md) | What lives where, and what is allowed to depend on what. |
| [Quality gates](development/quality-gates.md) | Every check that runs, what each one catches, and the measured performance budgets. |
| Testing | _planned_ — the testing strategy and how to run each suite |
| Building for iOS | _planned_ |
| Releasing | _planned_ |

## Conventions used here

- Paths are always relative to the repository root, like `crates/cairn-crypto/src/lib.rs`.
- Every command shown has been run. If something has not been verified, it is not here.
- Screenshots use invented sample data.
