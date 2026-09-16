# Cryptography

How a master password becomes the keys that protect everything, what each key is for, and what is written to disk to make it work again next time.

This page describes what exists today. Cairn is built in phases; the key hierarchy, the vault header and the session lock are finished, and the parts that consume the lower keys — the encrypted database, the export format and synchronisation — are not written yet. Where that is the case it is said so, rather than described as though it were done.

Everything here is verifiable against the source. The layout is in `crates/cairn-crypto/src/header.rs`, the hierarchy in `crates/cairn-crypto/src/hierarchy.rs`, and each claim below has a test named after it.

## What is used, and what is not

| Job | Primitive | Where |
| --- | --- | --- |
| Stretching the master password | Argon2id | `crates/cairn-crypto/src/kdf.rs` |
| Deriving one key from another | HKDF-SHA512 | `crates/cairn-crypto/src/hierarchy.rs` |
| Encrypting anything | XChaCha20-Poly1305 | `crates/cairn-crypto/src/aead.rs` |
| Random bytes | the operating system, through `getrandom` | `crates/cairn-crypto/src/random.rs` |

There is no other cipher, no other hash used for a security purpose and no other source of randomness. Nothing is implemented by hand: every primitive comes from the RustCrypto implementations, and the only code written here is the arrangement of them.

A test walks the source tree and fails if any crate other than `cairn-crypto` so much as names the cipher library. That is the enforcement behind the claim that encryption happens in one place; without it the claim would be a convention, and a convention survives until the first time somebody is in a hurry.

## The key hierarchy

```text
master password
  └─ Argon2id(password, salt, parameters from the header) ──► key encryption key
        └─ HKDF-SHA512(kek, "cairn/v1/wrap") ──► wrap key, which seals the data key

data key (thirty-two random bytes, wrapped in the header) ──► encrypts every record
  └─ HKDF-SHA512(dek, "cairn/v1/db")     ──► the raw key SQLCipher is given
  └─ HKDF-SHA512(dek, "cairn/v1/export") ──► encrypts a backup
  └─ HKDF-SHA512(dek, "cairn/v1/sync")   ──► the pre-shared key for synchronisation
```

The master password never encrypts anything directly. It is stretched into a key encryption key, that key is used for exactly one thing — unwrapping the data key — and the data key is what everything else hangs off.

The data key is thirty-two bytes from the operating system's random source. It is generated once, when the vault is created, and never changes for the life of the vault. That is the whole point of the arrangement: changing the master password derives a new key encryption key, re-wraps the same data key under it, and rewrites a hundred and sixty-eight bytes. Not one stored byte anywhere else has to be read, decrypted or written.

The same is true of changing the Argon2id parameters, which is the reason they can be raised later on hardware nobody has measured yet. That property has its own test: a vault is created, records are sealed under its data key, the parameters are changed, and every record still opens.

### Why the lower keys hang off the data key

This is a deliberate departure from the earlier design note, which had the database key derived from the key encryption key.

The key encryption key changes whenever the password or the parameters change. The data key does not. With the database key derived from the key encryption key, changing a master password would mean re-encrypting the entire database file — the most dangerous operation this application could ever perform, and one that would have to be performed correctly on a machine that might lose power halfway through. With it derived from the data key, changing a master password is a header rewrite.

What that costs is worth stating plainly. Anybody who holds the data key can already read every record, and both keys sit in the memory of the same process at the same time, so deriving the database key from one rather than the other does not widen what a successful attack yields. It is recorded as [decision 0003](decisions/0003-wrapped-data-key.md).

### Domain separation

Each derivation uses HKDF-SHA512 with a fixed `info` string, and the set of strings is a closed enumeration in the source rather than something a caller passes in. A typo in a string argument produces a key that works, is wrong, and collides with nothing until the day it collides with something.

The strings carry a version (`cairn/v1/...`). Without it, a future revision of this scheme deriving a key for the same purpose would derive the same key, which is a collision that looks like nothing is wrong. The exact bytes of every string are frozen by a test, because changing one silently would leave every vault that already exists unopenable.

HKDF is used with no salt. That is correct here rather than an omission: the specification defines the salt as a string of zeroes when none is given, and the input is already a uniformly random key rather than a password, so the extract step has nothing to do. The separation comes entirely from `info`.

## Argon2id

Argon2id is the only thing standing between a stolen file and an offline guessing attack on the master password. Everything else in the design is arithmetic that an attacker can perform as fast as their hardware allows.

The parameters live in the vault header, inside the part that is authenticated, and never as a constant in the code. A build that hard-coded them would be a build that could never raise them without abandoning every vault created before it.

The current defaults are 64 MiB of memory, three passes and one lane. The floor the code will accept is 32 MiB and three passes; a header asking for less is refused rather than honoured, because honouring it would let somebody who can edit the file make guessing cheap. The ceiling exists so that a header cannot demand more memory than a machine has.

These numbers are a starting point measured on desktop hardware. The phase that puts Cairn on a phone measures them there, and the fact that they can be changed afterwards without losing data is exactly why that measurement is allowed to happen later.

The password is normalised to Unicode NFC before it is hashed. Two keyboards can produce the same visible accented character as different byte sequences, and without normalisation the same password typed on two devices would derive two different keys.

## Authenticated encryption

Everything encrypted in Cairn is encrypted with XChaCha20-Poly1305, through two functions — `seal` and `open` — that are the only code in the project that touches a cipher.

A nonce is twenty-four bytes from the operating system's random source, fresh for every write. The extended nonce size is why it can be random rather than a counter: at that size the chance of a repeat is negligible for any number of writes this application will ever perform, and a counter would need state that survives restores from backup.

Reusing a nonce with the same key is catastrophic for this construction, so the design makes it impossible rather than discouraged. The nonce type can only be created by reading from the operating system, and it is consumed by value when it is used. A second use is a compile error. There is a test in `crates/cairn-crypto/tests/ui` that asserts the compiler really does refuse the program that tries.

Every ciphertext carries associated data that binds it to where it belongs. For the wrapped data key that is the whole authenticated prefix of the header: the salt, the parameters, the key identifier and both counters. Editing any of them breaks the unwrapping rather than cheapening the attack.

## The vault header

A hundred and sixty-eight bytes in a file of its own, beside the database rather than inside it. Putting it inside would be circular, because the database is encrypted with a key that hangs off the data key and the data key is what this file holds.

```text
offset  bytes  field
     0      8  magic, "CAIRNHDR"
     8      2  format version, currently 1
    10      2  reserved, must be zero
    12     16  salt for Argon2id
    28      4  Argon2id memory cost, in kibibytes
    32      4  Argon2id passes
    36      4  Argon2id lanes
    40     16  key identifier, a version four UUID
    56      8  when the vault was created, microseconds since the epoch, UTC
    64      8  when the parameters were last written, same units
    72      4  how many times the master password has been changed
    76      4  how many times this header has been rewritten
 -- 80: end of the authenticated prefix; these bytes are the associated data --
    80     24  nonce the data key was wrapped under
   104     48  the wrapped data key: thirty-two of key and sixteen of tag
 -- 152: end of everything the tag covers --
   152      4  failed unlock attempts          UNAUTHENTICATED
   156      8  locked until, same units        UNAUTHENTICATED
   164      4  CRC32 of the twelve bytes above UNAUTHENTICATED
```

The length is fixed and there is no length field anywhere in it. A parser with no length to trust and no loop to run is a parser with nowhere for an overflow to hide, and this is the one piece of input in the whole design that somebody who has stolen the file can rewrite at will. It is parsed by a total function and fuzzed continuously; the last run covered sixty-nine million inputs without a finding.

### The twelve bytes that are not authenticated

The failed attempt counter and the lockout moment cannot be authenticated, and the reason is structural rather than an oversight. They are written immediately after a failed unlock, which is exactly the moment when there is no key to authenticate with: re-wrapping the data key requires the password, and the password is the thing that was just got wrong.

So anybody holding the file can reset the counter to zero. This is written down here rather than glossed over. The CRC32 beside them is there to notice corruption, not tampering, and calling it anything else would be dishonest.

What that costs is close to nothing, because the counter was never the defence. The real cost of a guess is Argon2id, which charges its tenth of a second whether the counter exists or not, and anybody who has the file is attacking the file rather than sitting in front of the window. The counter makes the interface unpleasant to attack by hand; that is all it was ever for.

Everything that would actually help an attacker — the salt, the parameters, the key identifier — sits inside the authenticated prefix.

### Writing it safely

Every write goes to a temporary file, is flushed to the disk, and is renamed over the real one. The rename is a single step as far as the operating system is concerned, so after a power cut the file is either entirely the old one or entirely the new one and never half of each. The flush before the rename is what makes that promise real: without it the rename can be recorded while the contents are still in a buffer, and the result is a file of the right length full of nothing.

Both operations that rewrite the header — changing the password and changing the parameters — take a copy first, read it back and parse it before continuing. A copy nobody checked is a copy discovered to be useless at the one moment it was needed.

At startup the header is read, and if it cannot be parsed the copy beside it is used and the fact is reported on screen rather than handled quietly. If neither can be read, the application says so and refuses to create a new vault over the top, because creating one would make everything encrypted under the damaged header unreadable for good.

## Keys in memory

Keys never leave the Rust core. No key and nothing decrypted with one crosses into the WebView; the commands the interface can call return booleans, counts and enumerations.

Each key lives in a buffer that owns whole pages of memory and asks the operating system to keep them resident, so that a key is not written to the page file. Whole pages rather than a shared allocation, because both `VirtualLock` and `mlock` work on pages and neither is reference counted: locking the page that happens to contain a key would lock several dozen unrelated allocations, and unlocking it when that key is dropped would take the protection off any other key sharing it. Four kibibytes for a thirty-two byte key is worth paying, and a test asserts that two buffers never land on the same page.

This removes one copy and nothing else. It is explicitly not a defence against another process attached to this one, a crash dump, or hibernation. A machine that refuses to keep pages resident still opens its vault; the buffer reports what actually happened rather than what was intended.

Every key clears itself when it is dropped, and closing the vault is dropping it.

## What is not built yet

The database encryption, the export format and the synchronisation handshake all derive their keys from the hierarchy above, and none of them exists yet. The derivations are written and frozen by tests so that the format cannot drift, but nothing consumes them.

This page is updated as each one is built, rather than written in advance.
