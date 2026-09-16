# Getting started

From a fresh clone to the application running on your machine. Every command here has been run; if one of them does not work for you, that is a bug in this page.

## What you need first

**Rust.** Install it through [rustup](https://rustup.rs). Do not install a toolchain by hand and do not override the one this repository asks for: `rust-toolchain.toml` pins an exact version, and rustup installs it automatically the first time you run a cargo command here. Pinning is deliberate. A build that silently changes compiler between machines is not reproducible, and reproducibility is what lets somebody verify that a published binary corresponds to this source.

**Node.js**, at the version in `.nvmrc`, with the npm that ships with it. If you use a version manager, `nvm use` reads that file. The version is pinned for the same reason the Rust toolchain is, and the reason is not theoretical: npm's resolver changed between major versions, and an older npm refused to install a lockfile that a newer one had written and accepted. The failure appeared only in the pipeline, on a tree that had passed every check locally, which is the most expensive kind of difference to find.

**On Windows**, four more things. The Microsoft C++ build tools with the Windows 11 SDK, which you get by installing Visual Studio Build Tools and selecting the desktop C++ workload. The WebView2 runtime, which is already present on Windows 11; on anything older you have to install it from Microsoft. Without WebView2 the application will not start at all, and it cannot fall back to anything, because there is nothing to fall back to.

And then two that exist only because of the encrypted database: **Perl** and **NASM**, both on your `PATH`.

```
winget install --id StrawberryPerl.StrawberryPerl
winget install --id NASM.NASM
```

The database is SQLCipher, and SQLCipher needs OpenSSL. Neither is downloaded ready-made: both are compiled from C source as part of the build, on every machine, which is the only way to get one identical cipher on Windows and on a phone. OpenSSL's own configuration is a Perl program, so without Perl nothing builds at all. NASM assembles OpenSSL's hand-written x86-64 routines; without it OpenSSL still builds, but it falls back to a portable C implementation of AES that is both slower and, being table driven, more exposed to cache timing attacks than the AES-NI path. Install both.

The Perl that ships inside Git for Windows is not enough. It is a cut-down build missing modules OpenSSL's configuration needs, and it fails with `Can't locate Locale/Maketext/Simple.pm`, which reads like a broken installation rather than the wrong Perl. Install Strawberry Perl and make sure it comes first on `PATH`.

After installing either one, close every terminal you have open and start a new one. Windows only hands the updated `PATH` to processes started afterwards, and a build in an old window will keep saying the tool is missing.

**On Linux**, the WebKitGTK development packages Tauri links against. The [Tauri prerequisites page](https://tauri.app/start/prerequisites/) lists the exact package names for each distribution. Linux is not a target platform for the application itself, but the checks run there.

You will also want [gitleaks](https://github.com/gitleaks/gitleaks) on your `PATH`. The pre-commit hook refuses to run without it rather than skipping the scan.

## Setting up

```
git clone https://github.com/Necro-Coder/cairn.git
cd cairn
git config core.hooksPath .githooks
cargo fetch --locked
npm ci --ignore-scripts
```

Two of those lines deserve an explanation.

`git config core.hooksPath .githooks` is not optional. Git does not run hooks that live in the repository unless you point it at them, and this project relies on a pre-commit hook to keep secrets and personal paths out of a public history. The hook is at `.githooks/pre-commit`. Read it before you enable it; that is why it is in the repository rather than hidden in `.git/hooks`.

`--ignore-scripts` means no dependency gets to run a post-install script on your machine. It is how the project treats the npm supply chain everywhere, including in the pipeline.

## Running it

```
cargo tauri dev
```

The first build takes a while, and it is worth knowing why before you start wondering whether it has hung. It compiles the whole dependency graph, including the framework, and on top of that it compiles OpenSSL and SQLCipher from C. That part alone is several minutes on its own, and it happens once: cargo keeps the result and later builds reuse it.

It looks like nothing is happening because it mostly is not printing. The line to look for is `Compiling openssl-sys`, and while that one is on screen a C compiler is working through a few thousand source files without saying so. A hang looks different: no new output for a long time _and_ no processor activity. If `cl.exe`, `cc`, `perl` or `nasm` is busy in the task manager, it is building, not stuck.

After that first time it is fast, and the frontend hot-reloads while the window stays open.

You should get a window titled Cairn with a button. Press it, and the interface asks the Rust core for its name, version and build profile, and prints the answer. That is all the application does at this point, and proving that much works end to end is the entire purpose of it existing.

Press <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>D</kbd> for the diagnostics screen. <kbd>Esc</kbd> closes it.

If `cargo tauri` is not recognised, install the command line tool with `cargo install tauri-cli --version "^2" --locked`.

## Building a real binary

```
cargo tauri build -- --locked
```

The `--` matters: everything after it goes to cargo rather than to the Tauri tool, and `--locked` makes the build resolve exactly the versions in `Cargo.lock` and fail rather than quietly picking something newer.

This produces `target/release/cairn.exe` and an installer under `target/release/bundle/`. Neither is signed, and Windows will warn you about that the first time you run it. That warning is correct and you should treat every unsigned binary with the same suspicion, including this one.

## Checking your work

Run the gates before you commit. They are listed with what each one catches in [quality gates](quality-gates.md). The short version:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
npm run format:check
npm run lint
npm run check
```

## When something does not work

**`cargo tauri dev` fails to link on Windows.** The C++ build tools are missing or incomplete. Install the desktop C++ workload, including the Windows SDK, and open a new terminal so the environment is picked up.

**The window opens and stays blank.** The frontend dev server did not start, or it is not on the port the application expects. Check the terminal for a Vite error. The port is fixed at 1420 on purpose: if something else is already using it, the dev server fails loudly instead of moving to another port the window is not pointing at.

**The window never opens and there is no error.** On Windows this is usually a missing WebView2 runtime.

**The build stops in `openssl-sys` saying `Can't locate Locale/Maketext/Simple.pm`.** The Perl it found is the cut-down one inside Git for Windows. Install Strawberry Perl, put it ahead of Git's on `PATH`, and open a new terminal.

**The build stops in `openssl-sys` and the error mentions `nasm`.** NASM is missing from `PATH`. Install it and open a new terminal.

**The build sits on `Compiling openssl-sys` for minutes.** That is the expected behaviour, not a fault. See the note under _Running it_.

**A commit is rejected by the hook.** Read what it says; it names what it found. Do not reach for `--no-verify`. If the finding is wrong, fix the check, in the same commit, so the decision is visible.

**`npm run check` reports errors that `npm run lint` does not, or the other way round.** That is expected. They cover different things: `check` runs the Svelte compiler and is the authority on types inside components, and `lint` is the authority on the rules that keep the WebView safe. Both have to pass.
