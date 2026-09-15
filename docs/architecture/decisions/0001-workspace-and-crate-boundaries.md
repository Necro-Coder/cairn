# 0001 — A workspace of five crates, all created before they are needed

Date: 2026-09-15 · Status: accepted

## Context

Cairn is one application with three modules over a shared core, and it holds passwords, financial records and personal habits. Two properties matter more than anything else about how the code is arranged.

The first is that the part which must be right has to be small enough to read. Cryptography that is tangled with database access and window management cannot be reviewed on its own, and a reviewer who has to hold the whole application in their head to check a key derivation will not check it properly.

The second is that the repository is public specifically so that people can audit it. An auditor who has to trace through the whole application to find out whether the domain logic can reach the filesystem is an auditor who stops.

Two arrangements were available. Everything in `src-tauri/` as modules of one crate, which is what the framework's own template produces. Or a Cargo workspace with the core split into crates and the application as one more member.

There was a third question underneath: whether to create the crates now, while four of the five have nothing to put in them, or to create each one when the work arrives that needs it.

## Decision

A Cargo workspace with `crates/` at the root, alongside `src-tauri/` and `src/`.

Five crates, split by what they are responsible for rather than by technical layer: `cairn-crypto`, `cairn-domain`, `cairn-db`, `cairn-sync` and `cairn-platform`.

All five are created now, empty, compiling, each with a test.

Dependencies point inwards: `commands → domain → crypto`. The domain knows nothing about SQLite, about the framework, or about the format used on the wire. The cryptography crate performs no input or output at all.

`unsafe` is denied across the workspace and additionally forbidden in four of the five crates through an attribute that nothing inside them can turn off. `cairn-platform` is the exception, and containing that exception is the reason it exists as a separate crate.

## Alternatives considered, and why not

**Everything as modules of one crate.** This is simpler on the first day and it is what the template gives you. It was rejected because module boundaries inside a crate are not enforced by anything an auditor can check quickly. Rust's visibility rules make it easy for a module to reach a sibling, a `pub(crate)` item is visible to the whole crate, and nothing stops the domain from importing the database layer except somebody noticing in review. Crate boundaries are checked by the compiler, and the dependency graph is a file anyone can read.

**Creating each crate when the work arrives.** This was the more tempting alternative, because four empty crates look like ceremony. It was rejected on the grounds that a boundary is respected only if it already exists. When the crate is created at the moment something needs to cross into it, the crossing is exactly what motivates creating it, and it never gets uncrossed. Creating them first means the first commit that wants to violate the layering has to change a manifest to do it, which is visible in a diff in a way that an import statement is not.

**Splitting by technical layer rather than by responsibility**, with crates such as `models`, `services` and `utils`. Rejected because those names describe nothing. A crate called `utils` accumulates whatever has no other home and ends up depended on by everything, which is the opposite of a boundary. Every crate here can be described in one sentence that says what it is responsible for.

## Consequences

**Good.** The cryptography can be reviewed on its own, and it can be checked with property tests and under Miri because it has no input or output to mock. The domain is deterministic, with the clock and randomness injected, so its tests give the same answer on every machine. Unsafe code has one place to live and one place to audit, and a pipeline check asserts it has not appeared anywhere else. An auditor can read the dependency graph in the manifests instead of inferring it.

**Bad.** Five manifests to keep in step, which is mitigated by inheriting version, edition, licence and lints from the workspace, but is still five files. Cross-crate refactoring is more friction than moving code between modules. A type used by two crates needs a deliberate decision about which one owns it, rather than being moved without thought. And for the length of the first phase, four of the crates contain nothing but a version constant and a test, which anyone reading the repository will reasonably wonder about, which is why this document exists.

**Accepted.** The friction is the point. Every one of these costs is a moment where somebody has to decide, on purpose, to cross a boundary. In an application holding a password vault, that is the trade worth making.
