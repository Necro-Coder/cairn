# Threat model

Who Cairn is defending against, what each defence actually answers, and what it does not answer. The second half matters more than the first: a security document that only lists strengths is marketing.

This describes the system as it exists today — the cryptographic core, the vault header and the session lock. The database, the export format and synchronisation are not built yet, and the attackers who matter to them are named here without pretending the defences exist.

## What there is to lose

One person's passwords, financial records and personal habits, on their own devices. There is no server, no account and no other user. Nobody else's data is at risk if this fails, and nobody else's service goes down if it stops.

That shapes everything below. The asset is confidentiality of a single vault, and the realistic loss is not a breach of a system but somebody getting hold of one laptop or one phone.

## Where the boundaries are

```text
  ┌─ the machine ────────────────────────────────────────────────┐
  │                                                              │
  │   ┌─ WebView ───────────┐                                    │
  │   │ the interface       │ ──commands──► ┌─ Rust core ──────┐  │
  │   │ (Svelte, no runtime │               │ keys, cipher,    │  │
  │   │  dependencies)      │ ◄──status──── │ header, session  │  │
  │   └─────────────────────┘               └──────────────────┘  │
  │                                                  │            │
  │                                          ┌───────▼─────────┐  │
  │                                          │ cairn.header    │  │
  │                                          │ (168 bytes)     │  │
  │                                          │ database (later)│  │
  │                                          └─────────────────┘  │
  └──────────────────────────────────────────────────────────────┘
```

Three boundaries are worth naming.

**The disk.** Everything written is either encrypted or is the header, and the header is designed on the assumption that whoever holds it can rewrite it at will.

**The command boundary.** The WebView is the softest part of the architecture and the part an attacker reaches first. Nothing crosses it in the direction of the interface except booleans, counts and enumerations. No key, and nothing decrypted with one beyond the values being displayed at that moment.

**The process.** Keys exist in memory only while the vault is open, in pages the operating system is asked to keep resident, and are cleared when it closes.

There is no network boundary. Cairn makes no outbound connection of any kind. Synchronisation, when it exists, will be manual and over a private network the person set up themselves.

## The attackers

### Somebody who has the disk

The one the whole design is built around. A stolen laptop, a sold drive, a backup that ended up somewhere it should not have.

They have the header and, later, the database file. Both are useless without the master password, and getting the password means an offline guessing attack against Argon2id at the parameters recorded in the header. Lowering those parameters does not help them: the parameters are inside the authenticated prefix, so editing them breaks the unwrapping instead of cheapening the guess.

What they can do is reset the failed attempt counter, because the last twelve bytes of the header cannot be authenticated — writing them happens after a failed unlock, when there is no key to authenticate with. This costs almost nothing, because the counter was never the defence. It exists to make the interface unpleasant to attack by hand, and somebody holding the file left the interface behind a long time ago.

**Answered by:** Argon2id at parameters recorded in an authenticated header, a data key that is random rather than derived from anything guessable, and AEAD over everything.

**Not answered:** a weak master password. Nothing in this design saves a password that a word list contains. The interface says so and estimates strength, and it does not refuse on its own guess, because a program refusing a password on an opinion is a program people work around.

### Somebody sitting at an unlocked machine

The person walked away. This is the one the session lock exists for.

The vault closes itself after a period of inactivity that defaults to five minutes, where activity means keyboard or mouse inside this window and never system activity — somebody typing in another program is not somebody using this one. It closes at once when the window is minimised, and thirty seconds after the window loses focus. The thirty seconds is deliberate: switching away for a second to copy something happens constantly, and a vault that closed the moment it was not in front would become a vault whose automatic locking gets turned off.

The decision is taken inside the Rust core, on its own timer, not in the interface. A WebView that stopped reporting activity — or was made to stop — cannot hold the vault open.

**Answered by:** the inactivity timer, the focus and minimise rules, and keys that are cleared when the vault closes.

**Not answered:** somebody at the machine while it is genuinely in use. There is no second factor here and no attempt at one.

### Injected script inside the WebView

A vault entry whose title is markup, a backup file somebody else wrote, a future import path. If any of it is ever rendered as HTML, somebody else's file is executing code inside the window.

`{@html}` is forbidden across the whole frontend by a lint rule that fails the build, not by a convention. `eval`, `new Function`, `innerHTML` and `outerHTML` are banned the same way. The content security policy is strict and has no `unsafe-inline`. The frontend has no runtime dependencies at all — `dependencies` in `package.json` is empty and a pipeline check fails the build if it stops being so — so there is no third-party code in the bundle to be compromised.

If script does run despite all of that, what it reaches is the command surface, and that surface is small and enumerable in one file. It can ask to unlock the vault, which needs the password it does not have. It can ask to lock it. It cannot ask for a key, because no command returns one.

**Answered by:** a forbidden `{@html}`, an empty runtime dependency list, a strict policy, the Tauri isolation pattern, and a command surface that never returns key material.

**Not answered:** script that runs while the vault is open can read what is on screen, which is the same as what the person is looking at. There is no defence against that short of not displaying anything.

### Somebody who can edit the vault header

They can make the file unparseable, and they can reset the attempt counter. They cannot lower the Argon2id parameters, change the salt or swap the key identifier without the unwrapping failing, because all of it is inside the authenticated prefix.

The application refuses to create a new vault over a header it cannot read. That refusal is the single most important thing in this section: an application that offered to start fresh over a damaged header would be an application that destroys everything the person had, on their own instruction, in one click.

**Answered by:** an authenticated prefix that covers everything that matters, a total parser that is continuously fuzzed, a verified copy taken before every rewrite, and a header that is written whole and renamed into place so a power cut leaves one version or the other.

**Not answered:** deletion. Somebody who can delete the header has destroyed the vault, and there is no recovery from that except the person's own backup. The application says this out loud when the vault is created.

### A compromised dependency

Every package in the bundle runs with the same access as the application's own code.

The frontend has no runtime dependencies. The Rust side has the RustCrypto implementations and little else, with a committed lockfile, pinned versions, `cargo audit` and `cargo deny` as blocking gates, and an unused-dependency check.

**Not answered:** a compromise of a crate that is genuinely used, or of the compiler. Nothing here defends against that, and nothing pretends to.

## Explicitly out of scope

These are accepted limits, not oversights, and are repeated in [`SECURITY.md`](../../SECURITY.md) so that a report about one of them can be closed with a reference rather than an argument.

- An attacker with administrator or root privileges, or with a debugger attached to the process, while the vault is open.
- A hardware keylogger, a compromised keyboard, or a camera pointed at the screen.
- Physical coercion of the person.
- A jailbroken or rooted device.
- Denial of service against an application that runs locally for one person. If the process can be made to crash, it stops; nobody else is affected, and stopping is preferable to continuing with broken invariants.
- Traffic analysis, HTTP headers, cross-site request forgery and cross-origin policy. There is no server and no browser origin.

## Two things that are deliberately absent

**There is no account recovery.** No recovery phrase, no hint, no second door, no way for anyone to reset the master password. If it is lost, the data is gone. A recovery path is a second way in, and a second way in is a second thing to attack — usually the weaker of the two, and usually the one that does not need the password. It is recorded as [decision 0004](decisions/0004-no-recovery-and-one-unlock-error.md), and the interface says it in full before a vault is created rather than in a footnote afterwards.

**There is no difference between the ways an unlock can fail.** A wrong password, a header somebody edited and a flipped bit on a disk all produce the same error, with the same wording, in the same time. Telling them apart would tell an attacker which half of the problem to work on. The two exceptions are reported as themselves because neither says anything about whether the password was right: the length of the string that was just typed into a visible field, and this machine refusing to allocate the memory Argon2id asked for.

## What is not modelled yet

The database file, the export format and the synchronisation protocol. Each brings its own attackers — someone who can swap two rows, someone who supplies a hostile backup file, someone on the network between two devices — and each gets its section here when it is built, not before.
