# Cairn

A local-first personal app for habits, passwords and finances. Your data never leaves your devices.

> **Status: early development.** The cryptographic core is not finished and the app is not ready to hold data you care about. Do not use it as your password manager yet.

Cairn is one application with three modules built on a shared Rust core:

- **Habits** — daily and weekly habits, streaks, history, a year heatmap.
- **Vault** — an encrypted password manager with a generator, search, custom fields and secure notes.
- **Finances** — accounts, transactions, categories, budgets and monthly reports.

It runs on Windows and iPhone as a single native binary per platform, built with [Tauri](https://tauri.app). It is designed for one person and one person only: there are no accounts, no sign-up, no multi-user support.

## What it does not do

This list matters more than the feature list.

- **No servers.** Nothing is uploaded anywhere. There is no backend, no cloud fallback and no third-party service, not even for crash reports or analytics.
- **No network access** except one thing: a manual, user-initiated sync between your own two devices over your own private network. The app never opens a connection by itself.
- **No automatic update checks.** Updates are something you go and get.
- **No telemetry.** Ever.
- **No account recovery.** If you lose your master password, your data is gone. That is the point of the design, and it is why encrypted backups are a first-class feature rather than an afterthought.

## Security model in one paragraph

Your master password is stretched with Argon2id into a key-encryption key, which wraps a randomly generated data key, which encrypts your records with XChaCha20-Poly1305. The database file is encrypted as a whole, and sensitive fields are encrypted again individually, bound to the row they belong to so that records cannot be swapped or rolled back by anyone with write access to the file. No key and no decrypted text is ever handed to the WebView beyond what is on screen at that moment. Keys live in locked memory, are wiped on lock, and are never written to disk.

What it does **not** protect against is written down just as plainly in the threat model: an attacker with root access or a debugger attached while the app is unlocked, a hardware keylogger, or physical coercion. No user-space application defends against those, and any that claims to is lying.

## Documentation

Start at [docs/README.md](docs/README.md).

- **Using it** — installation, backups, syncing, and what each module does.
- **How it works** — architecture, threat model, cryptography, data model, sync algorithm and the iOS build pipeline.
- **Working on it** — getting a development environment running, the quality gates and the testing strategy.

## A note on language

The documentation is in English. The application's user interface is in Spanish, because it was written for its author. There is no translation layer yet.

## Installing it

There is nothing worth installing yet. When there is, the Windows build will be an unsigned executable, because signing it would mean paying for a certificate and this project costs nothing to run. Windows will show a SmartScreen warning the first time you open it. That warning is correct and you should treat every unsigned binary with the same suspicion, including this one: build it yourself from source if you would rather not trust a file you downloaded.

## Contributing and reporting problems

Bug reports and corrections are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first.

Security issues go through the private channel described in [SECURITY.md](SECURITY.md), never through a public issue.

## License

Licensed under the [Apache License 2.0](LICENSE).
