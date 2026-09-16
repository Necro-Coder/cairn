# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html) from the first release that a person can actually use. Until then the version stays in the `0.0.x` range and means nothing more than "this is early".

Entries are written for someone using the software, not for someone who wrote it.

## Unreleased

## 0.0.1 - 2026-09-15

The scaffolding release. There is no application to use yet: this is the repository, the build, and the checks that everything after it has to pass.

### Added

- An application shell that starts on Windows, reads its name and version from the Rust core and shows them on screen.
- A diagnostics screen, reachable with a keyboard shortcut, showing the version, the build profile, the operating system and the WebView version. It never shows a file path or anything derived from your data.
- Light and dark themes that follow the system setting.
- The Rust workspace the rest of the project is built on, split into the five parts that will hold cryptography, domain logic, storage, synchronisation and platform-specific code.
- A hardened WebView configuration: the isolation pattern, a content security policy with no inline or evaluated script, no global Tauri object and no asset protocol. A test reads the configuration and fails if any of that is loosened.
- Quality gates that run on every change: formatting, linting, type checking, tests, coverage, dependency advisories, licence policy, unsafe-code scanning and secret scanning.
- Apache-2.0 licence, a security policy with a private reporting channel, and contribution guidelines.

### Security

- No database is opened and no data is stored in this version. The cryptographic core does not exist yet, which is why the README says not to trust this with anything real.

There are no tags yet. Versions start being tagged once there is a module a person can use.
