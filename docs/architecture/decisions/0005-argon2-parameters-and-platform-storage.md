# 0005 — Argon2id parameters in the header, and the shape of platform storage

Date: 2026-09-17 · Status: accepted

## Context

Two decisions are recorded together here because they are the same kind of decision: both are promises made now to a phase that has not happened yet, and both would be expensive to break once there are real vaults on real machines.

The first is where the Argon2id parameters live. The numbers this project ships with — 64 MiB of memory, three passes, one lane — were measured on a desktop computer. The phone has not been measured. The plan for the phone is to lower them if the wait is too long, and that plan only works if the parameters are a property of the vault rather than a property of the build that created it.

The second is the shape of the interface to the operating system's own key store. Windows Hello sits in front of a credential backed by the platform module; Face ID and Touch ID sit in front of a key in the secure enclave; a Linux build has neither. Nothing is kept in any of them in this phase. The interface is nevertheless written now, because the phase that will keep something there is also the phase that hardens Windows, and discovering the shape of this interface while doing that would mean designing it under pressure from whichever platform happened to be written first.

## Decision

**The Argon2id parameters are written in the vault header, inside the authenticated prefix, and are read from it on every unlock.** There is no parameter constant that the unlock path consults. The only constant is the set used when a vault is first created.

**The parser enforces a floor and a ceiling before a single byte is allocated.** Memory must be between 32 MiB and 1 GiB, passes between three and sixteen, lanes between one and four. A header outside any of those ranges is refused, and the refusal happens while the numbers are still just numbers.

**There is no per-device calibration.** The same vault is opened on a desktop and on a phone, so the parameters are dictated by the slowest device that has to open it, and they are dictated once, by the vault.

**The starting point is high and moves down.** 64 MiB is above what the measurement is expected to need, so the first real measurement on a phone can only make the wait shorter. Starting low would mean a vault created today is stuck with a weak parameter set unless every one of them is rewritten.

**The secure storage interface has four operations**: keep a secret under a label, read it back, remove it, and declare what the store can promise. The declaration is a value with two fields — whether the secret is held by hardware, and which sensor guards it — and one question answered in one place: whether anything that opens the vault may be kept here at all.

**"This build has no store for this platform" is its own error variant**, distinct from "the store refused the authentication". Nothing collapses the two.

**Nothing is stored in any platform store in this phase.** The Windows implementation exists, works, and is called by nobody with a key.

## Alternatives considered, and why not

**Parameters as a constant in the code.** Simple, and wrong in a way that only shows up later: raising them for new vaults would leave every existing vault silently derived at the old cost, and lowering them for the phone would make every desktop vault weaker at the same time. The version of this that half works — a constant plus a migration — is the header design with extra steps and no authentication.

**Parameters in the header but outside the authenticated prefix.** This is the version that looks like it works. Somebody who can edit the file rewrites 64 MiB as 32 MiB, the unlock honours it, and guessing just got twice as cheap. Being inside the prefix means editing them breaks the unwrapping instead, so the attack turns into a denial of service against a file the attacker already had.

**A floor only, with no ceiling.** Rejected. A header claiming four gigabytes is hostile input that costs nothing to write and takes the process down by allocation before any authentication has happened. The ceiling is not about cryptography, it is about a parser refusing to act on a number it has not checked.

**A ceiling only, with no floor.** Rejected for the mirror reason. Without a floor, the parameters can be edited down to something OWASP would not accept and the vault opens as if nothing happened.

**Calibrating on first run.** Rejected. A desktop that calibrates to a gigabyte produces a vault the phone cannot open, and the alternative — keeping several wrappings in parallel, one per device class — multiplies the cryptographic surface to solve a problem that having one number does not have.

**Adding `exists` and `clear_all` to the storage interface.** Rejected. `exists` is `retrieve` returning nothing, and a caller that asks before reading has written a race. `clear_all` is an operation whose blast radius is every label, including ones another part of the application owns.

**Leaving the capabilities out.** Rejected, and this is the one that would have hurt most. Without them the layer that decides what may be kept has to ask what platform it is on, which means that decision is spread across every call site and has to be revisited whenever a platform is added. With them, a store that cannot promise hardware backing simply says so, and one question at one call site refuses to hand it anything.

**One error for "not implemented" and "authentication failed".** Rejected explicitly, with a test. A build with no store would be indistinguishable from a person whose fingerprint was not read, which produces the worst possible behaviour: the application asks for the master password believing the sensor failed, and is right for the wrong reason. Where the two are told apart, one means stop offering this and the other means offer it again.

## Consequences

**Good.** Changing the parameters is a supported operation rather than a migration: the key encryption key is re-derived, the data key is re-wrapped, and 168 bytes are rewritten. No record is re-encrypted, and there is a test that proves it by checking the data key identifier is unchanged either side of the change.

**Good.** The phone measurement in a later phase is now a measurement rather than a commitment. Whatever it finds can be applied to real vaults with real data in them.

**Good.** The parser is total over its input in the part that matters: every parameter is range-checked before anything acts on it, and the fuzzing target covers the header.

**Bad.** The floor and the ceiling are judgement calls that are now baked into the accepted range, and widening them later is a format question rather than a preference. The ceiling in particular will look small on a machine that has plenty of memory.

**Bad.** The storage interface is designed against three platform stores, one of which is not written yet. It may turn out to be the wrong shape for the fourth. That risk is accepted and bounded: it is four functions, and nothing depends on it yet.

**Neutral.** The Windows implementation exists with no caller, which reads like dead code and is not: it is the half of the contract that can be tested today, and the phase that gives it something to hold is the phase that puts Windows Hello in front of it.
