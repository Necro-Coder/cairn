# Contributing

Cairn is a personal project built in the open. The code is public so that it can be audited, not because the project is looking for feature contributions. Bug reports, security findings and corrections to the documentation are genuinely welcome. A large pull request that adds a feature nobody asked for will probably be declined, so please open an issue first and let us agree on the shape of the change before writing it.

## What you need

- Rust, installed through [rustup](https://rustup.rs). The exact version is pinned in `rust-toolchain.toml` and rustup will install it automatically the first time you run a cargo command in this repository. Do not override it.
- Node.js 20 or newer, with npm.
- On Windows: the Microsoft C++ build tools with the Windows SDK, and the WebView2 runtime. WebView2 ships with Windows 11; on older systems you have to install it.
- On Linux: the WebKitGTK development packages that Tauri requires. The [Tauri prerequisites page](https://tauri.app/start/prerequisites/) lists the exact package names per distribution.

## Getting set up

```
git clone https://github.com/Necro-Coder/cairn.git
cd cairn
git config core.hooksPath .githooks
cargo fetch --locked
npm ci --ignore-scripts
```

The `core.hooksPath` line is not optional. Git does not run hooks that live in the repository unless you point it at them, and this project relies on a pre-commit hook to keep secrets out of the history. The hook is in `.githooks/pre-commit` and you are encouraged to read it before enabling it.

`npm ci --ignore-scripts` is deliberate. No dependency gets to run a post-install script on your machine.

Then run the application:

```
cargo tauri dev
```

The full walkthrough, including what to do when something does not work, is in [docs/development/getting-started.md](docs/development/getting-started.md).

## Quality gates

Every one of these has to pass before a pull request can be merged, and all of them run in CI. Run them locally first; CI is a safety net, not a substitute for checking your own work.

| What | Command |
| --- | --- |
| Rust formatting | `cargo fmt --all -- --check` |
| Rust lints | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| Rust tests | `cargo test --workspace --all-features` |
| Rust coverage | `cargo llvm-cov --workspace --all-features --fail-under-lines 80` |
| Frontend formatting | `npm run format:check` |
| Frontend lints | `npm run lint` |
| Frontend types | `npm run check` |
| Dependency licences and advisories | `cargo deny check` |
| Rust advisories | `cargo audit` |
| npm advisories | `npm audit --omit=dev` |
| Unsafe code | `cargo geiger --forbid-only` |
| Secrets | `gitleaks detect --redact` |

What each one catches, and what to do when it fails, is in [docs/development/quality-gates.md](docs/development/quality-gates.md).

A check that was not run is never reported as passing. If you could not run something, say so and say why.

## Rules that are not negotiable

These come out of the threat model. A pull request that breaks one of them will be declined regardless of how good the rest of it is.

- No key and no decrypted text crosses the boundary into JavaScript. Cryptographic material lives in the Rust core and nowhere else.
- `unsafe` is forbidden everywhere except `crates/cairn-platform`, which exists precisely so that the unsafe code has one place to live and one place to audit. Every `unsafe` block carries a `// SAFETY:` comment justifying each invariant it relies on.
- `{@html}` is forbidden in the frontend, and the linter enforces it. An imported vault entry is hostile input; rendering it as HTML is a path from a malicious backup file to the command boundary.
- `src/lib/ipc.ts` is the only file that imports `invoke`. One file to audit instead of fifty.
- Floating point is never used for money.
- No `unwrap`, `expect`, `panic!` or unchecked indexing outside tests, unless the line carries a comment proving it cannot fail.
- No network access beyond the manual synchronisation feature. No telemetry, no crash reporting, no update checks, no third-party services.

## Adding a dependency

Adding a dependency is a security decision, not a convenience decision. Before you propose one, answer these in the pull request: can the standard library or twenty lines of our own code do it, is it actively maintained, does it have open advisories, how many transitive dependencies does it drag in, is its licence compatible, and does it run anything at install time.

Cryptographic dependencies are limited to the RustCrypto family, which has public audits. The frontend has no runtime dependencies at all, and that is a property to preserve: `dependencies` in `package.json` is meant to stay empty.

## Commits

[Conventional Commits](https://www.conventionalcommits.org), with the crate or module as the scope.

```
feat(crypto): derive per-purpose subkeys with HKDF-SHA512

The key-encryption key is no longer used directly. Every consumer gets a
subkey with its own info string, so a flaw in the synchronisation channel
cannot compromise the key that protects the database file.
```

- The subject line is in English, imperative, and no longer than 72 characters. The body explains why, not what.
- One logical change per commit. The repository compiles and the tests pass at every commit.
- Never `--no-verify`. If a hook fails, fix the cause.
- No automatic attribution, co-authorship or signature lines of any kind.

Commits are signed. If you are contributing, signing yours is appreciated but not required.

## Branches and pull requests

Branch names follow `<type>/<slug>`, using the same types as the commit convention.

Pull requests are written in English, because the repository is public. Describe what the change does, why it is done this way, how to test it by hand, what could go wrong and how to undo it. Include the table of gates you ran with their real results.

If the change touches cryptography, the database schema, the command boundary, import and export, or synchronisation, it needs a security review written into the pull request, not just a green pipeline.

## Language

Everything written down is in English: identifiers, comments, documentation, commit messages, issues and pull requests. The repository is public so that it can be audited, and a comment an auditor cannot read is a comment that does not exist.

The single exception is the application's user interface, which is in Spanish because it was written for its author. There is no translation layer yet.
