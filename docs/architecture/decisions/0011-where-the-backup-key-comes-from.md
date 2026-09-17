# 0011 — The backup key hangs off the key encryption key, not off the data key

Date: 2026-09-17 · Status: accepted

## Context

[Decision 0003](0003-wrapped-data-key.md) gives this application two keys and a rule. The master password goes through Argon2id to a key encryption key, the `KEK`. A random data key, the `DEK`, is generated once and stored wrapped under the `KEK`. Everything else — the database key, the field key, the sync key — is derived from the `DEK` through HKDF with a domain separation label. That indirection is what lets the master password and the Argon2id parameters change without re-encrypting a single stored byte.

A backup needs a key too, and the shape of the hierarchy makes `Purpose::Export` off the `DEK` the obvious place to put it. That is what the original design said and what the code did.

It is wrong, and the reason only becomes visible when you ask what a backup is for.

A backup that can be sealed with a key derived from the `DEK` is a backup that can be made by anything holding the `DEK` — which is to say, by the process, at any moment the vault is open, without a password. More importantly it inverts the property the whole feature exists for. The `DEK` is reachable only by unwrapping it with the `KEK`, and the `KEK` comes from the password; so the key is downstream of the password in theory. But `UnlockedVault` holds the derived keys, not the `KEK`, so in practice the export key was reachable from a value that outlives the password by the whole length of a session.

The question that settles it: a person exports a backup, then changes their master password, then loses their machine. Which password opens the file? Under `Purpose::Export` off the `DEK`, the answer is neither, exactly — the file is sealed with a key that hangs off a random value stored in a header file they no longer have. Under Argon2id over the backup's own salt with the password typed at export time, the answer is the one they typed, and nothing else is needed.

## Decision

**`export_key` takes a `Kek`, not a `DataKey`.** Same HKDF label, same construction, different input. The signature is `export_key(kek: &Kek) -> DataKey`.

**`UnlockedVault::export_key` is deleted.** Not deprecated, not left behind a flag: removed, so that the open vault has no method that hands back anything a backup could be sealed with. The module documentation says why it is absent and points here, because an absence is invisible and somebody will otherwise add it back as an oversight.

**The key a backup file is actually sealed with is derived from the password that is typed at the moment of export**, through Argon2id over the salt and the parameters written into that file's own header — not from the vault's `KEK`, and not from anything in memory. The function above is what gives that derivation its domain separation; it is not a way to reach the file's key from the session.

**The verification pass derives the key again from the finished file's header** rather than reusing the one it just used to write. Otherwise the check that the file is readable never reads the header it is checking.

**The password may be the master password or a different one chosen for the file.** When it is the master password, the core verifies that it really is, with a full derivation against the vault header that is then discarded. When it is a separate one, it is checked against the password policy.

## Alternatives considered, and why not

**`Purpose::Export` off the `DEK`, as originally designed.** Rejected for the reason above: it makes a backup openable only by somebody who has the `DEK`, which means the vault header from the machine that made it. A backup that needs the machine it came from is not a backup.

**Sealing the backup with the `DEK` itself and storing it wrapped in the file.** This is what the vault header does, and it is right there — one wrapped key and the file opens with the master password. It fails the same test: after a password change, the old file still opens with the old password, and somebody who remembers only the new one has a file nobody can open. Worse, it is not obvious that this is what happened, so they find out by trying.

**Keeping `UnlockedVault::export_key` and simply not calling it.** Rejected. A method that exists is a method that gets called. The point of removing it is that no future phase can write an export sealed with something reachable from an open vault without first noticing that the method is gone and reading why.

**Not verifying the master password, and just deriving with whatever was typed.** It is cheaper — a whole Argon2id run cheaper — and it means a typo produces a valid backup whose password nobody knows. Nothing detects that. The verification pass would pass, because it derives from the same typo. Rejected: one second at export against a file discovered to be useless in a year.

**Letting the frontend decide whether the password is the master one.** Rejected on the same principle as everywhere else in this application: the check is cryptographic and the core does it. The interface offers the choice; the core proves it.

## Consequences

**Good.** A backup opens with a password. Not with a machine, not with a header file, not with an installation of this application. That is the whole point of the feature and it is now true by construction rather than by care.

**Good.** Changing the master password does not silently orphan existing backups, because a backup was never sealed with anything derived from the vault's own stored key material. It stays openable with the password it was made with, which is a fact somebody can be told in one sentence.

**Good.** There is no path from an open vault to a backup key. An attacker who gets code running in the process with the vault open still has to make the person type a password to get an exportable file out of it.

**Bad.** An export costs two Argon2id runs at the file's parameters plus one against the vault header, rather than a cheap HKDF. On the default parameters that is a few seconds before any bytes are written. It is paid once per export and it is reported as progress.

**Bad.** Somebody who exports under a separate password and forgets it has lost that file, completely, with no recovery path, in exactly the way [decision 0004](0004-no-recovery-and-one-unlock-error.md) describes for the vault itself. The screen says so above the fields rather than under them.

**Neutral.** `Purpose::Export` still exists and still has its own HKDF label, so the derivation is still domain-separated from the database key, the field key and the sync key. What changed is what goes in, not what comes out.
