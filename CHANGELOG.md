# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html) from the first release that a person can actually use. Until then the version stays in the `0.0.x` range and means nothing more than "this is early".

Entries are written for someone using the software, not for someone who wrote it.

## Unreleased

### Added

- A vault. You choose a master password, and the application creates the keys that everything stored later is encrypted with. It tells you before you start that there is no way to recover that password, and asks you to acknowledge it.
- Opening the vault, and closing it. It closes by itself after five minutes without keyboard or mouse inside the window, at once when the window is minimised, and thirty seconds after the window loses focus. How long the inactivity period is can be changed, including turning it off, which comes with a warning.
- Changing the master password, and changing how much work the application does to turn it into a key. Neither re-encrypts anything you have stored: both rewrite the small file that holds the keys, after taking a copy of it and reading the copy back.
- A wait after a wrong password that doubles each time, from one second up to five minutes. It survives restarting the application.
- A strength estimate while you are choosing a password. It is an estimate and it never refuses a password on its own opinion.
- Recovery of the key file if the application is interrupted while writing it. If the file cannot be read at startup and a copy is available, the copy is restored and you are told. If the file is damaged and there is no copy, the application says so and will not offer to create a new vault over it.

### Security

- Keys exist only while the vault is open, in memory the operating system is asked not to write to disk, and are cleared when it closes.
- No key and nothing decrypted with one ever leaves the Rust core. The interface receives what it is displaying and nothing else.
- Every way an unlock can fail produces the same message, so it says nothing about which part of the problem to attack.
- The parameters that control the cost of guessing your password are stored inside the part of the key file that is authenticated, so they cannot be lowered by editing the file.

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
