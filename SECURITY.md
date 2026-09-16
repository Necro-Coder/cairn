# Security

Cairn stores passwords, financial records and personal habits on a single person's devices. That is the whole reason the project exists, and it is also the reason this file is not a formality.

## Reporting a vulnerability

Report privately through [GitHub's private vulnerability reporting](https://github.com/Necro-Coder/cairn/security/advisories/new). That channel is enabled on this repository and is the only one that should be used for security issues.

Please do not open a public issue for a vulnerability, and please do not post a proof of concept publicly before a fix exists.

A useful report contains the affected version or commit, what an attacker gains, the steps to reproduce it, and the assumptions the attacker needs to satisfy. A minimal proof of concept is welcome; a weaponised exploit is not needed and is not wanted.

Expect an acknowledgement within a week. This is a one-person project with no service-level agreement behind it, so the honest answer is that response times depend on the severity and on how much free time exists that week. Critical findings in the cryptographic core, the storage layer or the command boundary jump the queue.

There is no bug bounty. There is no money in this project at all.

## What is in scope

- The cryptographic core: key derivation, the key hierarchy, authenticated encryption, nonce handling and the data bound to each ciphertext.
- The storage layer: how the database file is encrypted, and what an attacker with write access to that file can change without being detected.
- The command boundary between the WebView and the Rust core, including anything that would let injected JavaScript reach a command it should not reach.
- The backup and import format, including malformed or hostile files.
- The synchronisation protocol: the handshake, the framing and the merge algorithm.
- Platform hardening: memory handling, the clipboard, screen capture and the way secrets are stored by the operating system.
- The build and release pipeline, including anything that would let a third party influence the produced binary.

## What is out of scope

These are not oversights. They are stated in the threat model as accepted limits, and a report about one of them will be closed as out of scope.

- An attacker who already has administrator or root privileges on the machine, or who has a debugger attached to the process, while the application is unlocked.
- A hardware keylogger, a compromised keyboard, or a camera pointed at the screen.
- Physical coercion of the user.
- A jailbroken or rooted device running the application.
- Denial of service against an application that runs locally for a single user. If the process can be made to crash, it stops; nobody else is affected, and a crash is preferred over continuing with broken invariants.
- Missing HTTP security headers, CSRF and CORS findings. There is no server and no browser origin to attack.
- Reports produced by a scanner with no analysis attached, and findings about dependencies that are not reachable from any code path in this project.

## The short version of the threat model

The master password is stretched with Argon2id into a key that wraps a randomly generated data key, and that data key encrypts records with XChaCha20-Poly1305. The database file is encrypted as a whole, and sensitive fields are encrypted again individually. Each ciphertext is bound to the row it belongs to, so that a record cannot be moved, swapped or rolled back by someone with write access to the file without the decryption failing.

No key and no decrypted text crosses into the WebView beyond the values being displayed at that moment. Keys are held in locked memory, wiped when the application locks, and never written to disk.

There is no account recovery. If the master password is lost, the data is gone. That is a design decision, not a missing feature.

There is no account recovery and no way to tell why an unlock failed. Both are deliberate and both are argued in full in [decision 0004](docs/architecture/decisions/0004-no-recovery-and-one-unlock-error.md).

The full description lives in [the cryptography page](docs/architecture/cryptography.md) and [the threat model](docs/architecture/threat-model.md), which are kept in step with what is actually built rather than written in advance. Both are explicit about the parts that are not defended, including the twelve bytes of the vault header that cannot be authenticated and what that costs.

## Supported versions

The project is in early development and has not had a release yet. Only the current state of the default branch is supported. Nothing published so far is intended to hold data anyone cares about.

## Disclosure

Once a fix exists, the advisory is published with credit to the reporter unless they prefer otherwise, and a regression test is added so the same issue cannot come back unnoticed.
