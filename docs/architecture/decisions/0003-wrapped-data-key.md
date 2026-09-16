# 0003 — A wrapped data key, with every other key derived from it

Date: 2026-09-16 · Status: accepted

## Context

A master password has to end up protecting a database, exported backups and a synchronisation channel. There are two shapes this can take.

The direct one derives each key from the master password. The indirect one stretches the password into a key encryption key, uses that key for exactly one thing — unwrapping a randomly generated data key — and derives everything else from the data key.

The difference only shows up when something about the password changes, and two things will change. The master password itself, because people change passwords. And the Argon2id parameters, because the numbers this project ships with were measured on desktop hardware and the phone has not been measured yet. The whole plan for that phone depends on being able to raise the parameters afterwards.

With the direct shape, either of those changes every key in the system. Changing a master password would mean re-encrypting the entire database. That is the most dangerous operation this application could ever perform: it reads and rewrites everything the person owns, it has to be correct on a machine that might lose power halfway through, and if it goes wrong there is no recovery path, by design.

A further question sits inside the indirect shape, and it is the one that took the actual deliberation. The key handed to SQLCipher for the database file can be derived from the key encryption key or from the data key. The earlier design note for this project said the key encryption key.

## Decision

The indirect shape, and every key below the wrapping derives from the **data key**, not from the key encryption key.

```text
master password
  └─ Argon2id(password, salt, parameters from the header) ──► key encryption key
        └─ HKDF-SHA512(kek, "cairn/v1/wrap") ──► wrap key, which seals the data key

data key (thirty-two random bytes, wrapped in the header) ──► encrypts every record
  └─ HKDF-SHA512(dek, "cairn/v1/db")     ──► the raw key SQLCipher is given
  └─ HKDF-SHA512(dek, "cairn/v1/export") ──► encrypts a backup
  └─ HKDF-SHA512(dek, "cairn/v1/sync")   ──► the pre-shared key for synchronisation
```

The data key is thirty-two bytes from the operating system's random source, generated once when the vault is created, and it never changes for the life of the vault. Changing the master password or the Argon2id parameters derives a new key encryption key, re-wraps the same data key under it and rewrites a hundred and sixty-eight bytes of header. Nothing else on disk is read or written.

The `info` strings are a closed enumeration in the source rather than arguments a caller passes, and they carry a version. Their exact bytes are frozen by a test.

## Alternatives considered, and why not

**Deriving every key straight from the master password.** Rejected for the reason above: it makes a password change into a full re-encryption of everything the person owns, in an application that has no recovery path if that re-encryption goes wrong. It also makes raising the Argon2id parameters cost the same, which would mean the numbers chosen today were effectively permanent.

**Deriving the database key from the key encryption key**, as the project's earlier design note had it. This is the alternative that was genuinely close, and it was rejected after the trade was written out.

Its argument is layering: the database key is one step further from the data key, so a hypothetical compromise of the data key alone would not yield the database. That argument does not survive contact with this system. Anybody holding the data key can already read every record, because the data key is what records are encrypted under. Both keys live in the memory of the same process at the same time. There is no scenario in this design where an attacker obtains one and not the other, so the extra layer buys nothing measurable.

What it costs is very measurable. With the database key hanging off the key encryption key, changing the master password changes the database key, and changing the database key means `PRAGMA rekey` over the whole file — exactly the operation the indirect shape existed to avoid. The layering would have reintroduced the problem it was chosen to solve.

**Keeping the data key in the platform's secure store** instead of wrapping it in the header. Rejected for now because the only store available on the platform being built first is a file protected by whatever protects the user account, which on a stolen disk is nothing. Writing the wrapped key there would hand anybody with the drive a copy of the vault. The interface for a secure store exists and reports honestly that it may not hold vault material; when a hardware-backed store arrives, that flag turns true in one place.

**A key hierarchy with more levels**, separating for example read keys from write keys. Rejected as complexity with no attacker behind it. One person, one device at a time, no roles.

## Consequences

**Good.** Changing the master password costs a hundred and sixty-eight bytes. Changing the Argon2id parameters costs the same, which is what makes it safe to defer measuring them on a phone until there is a phone. `PRAGMA rekey` does not appear anywhere in this project, and neither does any other operation that rewrites the whole database. A test creates a vault, seals records under it, changes the parameters and asserts every record still opens.

**Bad.** The data key is a single point of failure for the life of the vault. It cannot be rotated without re-encrypting everything, which is precisely the operation this decision avoids, so rotating it is not offered. If it is ever disclosed, changing the master password does not help.

**Bad.** This is a departure from the project's own earlier design note, which is now wrong where it describes the database key. That note is private; this record is the public statement of what was actually built, and the divergence is the reason this record exists.

**Neutral.** The header becomes load-bearing in a way it would not otherwise be. Losing it loses the vault, so it is written atomically, copied and verified before every rewrite, and recovered from that copy at startup. That machinery would have been needed anyway, but this decision is what makes it essential rather than merely prudent.
