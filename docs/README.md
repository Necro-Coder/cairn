# Cairn documentation

Three ways in, depending on what you want.

- **[Using Cairn](#using-cairn)** — you want to install it and use it.
- **[How it works](#how-it-works)** — you want to understand, audit or verify it.
- **[Working on Cairn](#working-on-cairn)** — you want to build it or contribute.

> The project is in early development. Pages marked *planned* do not exist yet; they are written as the feature they describe is finished, so that nothing here describes something that is not real.

## Using Cairn

| Page | What it covers |
|---|---|
| Installation on Windows | *planned* |
| Installation on iPhone | *planned* |
| Getting started | *planned* — creating your master password and first records |
| Security model | *planned* — what Cairn protects, and what it does not, in plain words |
| Backup and restore | *planned* — how encrypted exports work and why you need them |
| Syncing your devices | *planned* |
| Habits | *planned* |
| Vault | *planned* |
| Finances | *planned* |
| Troubleshooting | *planned* |

The user interface is in Spanish. This documentation is in English. See the note at the end of the [project README](../README.md).

## How it works

| Page | What it covers |
|---|---|
| Overview | *planned* — the whole system in one page, read this first |
| Threat model | *planned* — who the attacker is and what each defense answers |
| Cryptography | *planned* — key hierarchy, AEAD, nonces, associated data |
| Data model | *planned* — the schema and why every column exists |
| Storage | *planned* — the two layers of encryption at rest |
| Backup format | *planned* — the export file, byte by byte |
| Sync | *planned* — transport, handshake and the merge algorithm |
| Platform hardening | *planned* — operating system side channels on Windows and iOS |
| iOS pipeline | *planned* — how the app is built and signed without a Mac |
| Decision records | *planned* — the choices that are expensive to reverse |

Two things this section will always do: explain the reasoning rather than restate the code, and be explicit about the weaknesses. A security document that only lists strengths is marketing.

## Working on Cairn

| Page | What it covers |
|---|---|
| Getting started | *planned* — from a fresh clone to a running app |
| Project layout | *planned* — what lives where and what depends on what |
| Quality gates | *planned* — every check that runs, and what each one catches |
| Testing | *planned* — the testing strategy and how to run each suite |
| Building for iOS | *planned* |
| Releasing | *planned* |

## Conventions used here

- Paths are always relative to the repository root, like `crates/cairn-crypto/src/lib.rs`.
- Every command shown has been run. If something has not been verified, it is not here.
- Screenshots use invented sample data.
