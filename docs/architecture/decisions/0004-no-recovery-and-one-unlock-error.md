# 0004 — No recovery path, and one indistinguishable unlock failure

Date: 2026-09-16 · Status: accepted

## Context

Two questions come up the moment a master password protects something, and they are usually answered separately. They are recorded together here because they are the same answer applied twice: every additional thing the program is willing to tell you, or willing to do for you, is a thing an attacker can use.

The first is what happens when somebody forgets the master password. Every product that holds data for people has an answer to this, and most of them are some form of recovery: a phrase written down at setup, a second factor, an escrow key held by the vendor, a set of security questions.

The second is what the application says when a vault does not open. A vault can fail to open because the password was wrong, because somebody edited the header, because a bit flipped on a disk, or because the file was truncated by a power cut. Telling them apart is helpful. It is also information.

## Decision

**There is no recovery path.** No recovery phrase, no hint, no security questions, no escrow, no vendor-held key, no administrative override. If the master password is lost, the data is unreadable and stays unreadable.

The interface says this in full, before the fields rather than after them, on the screen that creates the vault, and it will not proceed until the person has acknowledged it explicitly.

**Every way an unlock can fail produces the same error.** A wrong password, an edited header, a flipped bit and a truncated file all return a single value with the same wording. The work done before returning it is also the same: the derivation runs in every one of those cases, because what distinguishes them is discovered when the unwrapping is attempted afterwards.

Two outcomes are reported as themselves, because neither says anything about whether the password was right. One is the length of the string that was just typed into a field the person can see. The other is this machine refusing to allocate the memory Argon2id asked for, which is a fact about the machine.

The lockout after repeated failures is also reported separately, and it is decided **before** anything is derived, so it says nothing about the password either.

## Alternatives considered, and why not

**A recovery phrase generated at setup.** The obvious one, and the one most products choose. Rejected because it is a second key that opens everything, and it is the weaker of the two: it is written on paper, or in a photo, or in another password manager, and it is never changed. An attacker who knows a recovery phrase exists attacks the phrase, not the password. The honest summary is that offering one would move the security of the vault from "as strong as a password stretched with Argon2id" to "as strong as wherever that piece of paper ended up", while letting everyone believe the first number.

**Escrow with the vendor.** There is no vendor. There is no server and no account. Building one to hold recovery keys would create the exact centralised target this project exists to avoid.

**A second factor that can open the vault on its own.** Rejected for the same reason as the recovery phrase: it is a second door. A second factor that is required _in addition to_ the password is a different proposal and is not ruled out by this record; it is simply not built.

**Security questions.** Rejected. They are a password chosen from a small set of guessable answers, and the answers are frequently public.

**Distinguishing the unlock failures.** Rejected after working through what it gives an attacker. Somebody with the file who is told "the header is malformed" rather than "wrong password" learns that their edit was detected, which tells them which bytes are authenticated and lets them search for ones that are not. Somebody told "wrong password" specifically learns that the file itself is intact and that guessing is the only remaining avenue, which is worth knowing before spending a week of compute. Neither is catastrophic on its own, and both are free to remove.

**Distinguishing them only where the interface cannot see them.** The cryptographic core does keep the distinction in its own error type, and the layer that answers the interface is where it is collapsed. That is not an alternative to this decision but part of it: the distinction is useful to whoever is fixing a broken machine, and it is an oracle in a value a caller can read. There is no logging in the application yet, so today the distinction is only visible to somebody with a debugger or a test.

## Consequences

**Good.** There is exactly one way into a vault, so there is exactly one thing to analyse and one thing to get right. The claim "the security of this vault is the security of the master password" is literally true rather than approximately true.

**Good.** The unlock path takes the same route whatever the reason, so there is one fewer place for a timing difference to appear and one fewer thing for a future change to reveal by accident. What branching remains is between categories that are not about the password at all: the length of what was typed, and this machine refusing to allocate.

**Bad, and it will happen.** Somebody will lose their master password and lose their data. There is no mitigation for this beyond saying so clearly and early, which the interface does. This is the single worst consequence in the whole project and it is accepted knowingly.

**Bad.** Diagnosing a genuinely corrupt vault is harder from the interface alone, because the interface is not allowed to say what is wrong. What it does report is what it can report safely and what is not an oracle: whether a copy was restored at startup, and whether the header could be read at all. Neither of those is discovered by attempting a password.

**Neutral.** The single error type has to be enforced rather than assumed. There is a test that walks every failure the cryptographic core can produce and asserts they all map to the same value, so that a variant added later cannot quietly become distinguishable.
