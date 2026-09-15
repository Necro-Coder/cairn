# 0002 — Svelte, hand written CSS, and no runtime dependencies

Date: 2026-09-15 · Status: accepted

## Context

The interface runs in a WebView, which is the softest part of this architecture. There is no server and no browser origin, so some familiar web problems do not apply here: there is no cross-site request forgery to worry about and no cross-origin policy to get wrong. Two very much do apply, and they are the realistic route to compromising a password vault.

The first is cross-site scripting. The application will import backup files and, later, receive records from another device. A vault entry whose title is markup is hostile input, and if it is ever rendered as HTML then somebody else's file is executing code inside the window.

The second is the supply chain. Every npm package that ends up in the bundle is code running with the same access as the application's own code. It can read anything on screen and it can call anything the boundary exposes. A compromised transitive dependency of a date picker is indistinguishable from a compromise of the application itself.

That second point is not hypothetical for the npm ecosystem, and the number of packages in a typical frontend is large enough that nobody reads them.

## Decision

Svelte 5 with Vite and TypeScript.

No runtime dependencies. `dependencies` in `package.json` is empty, and a pipeline check fails the build if it stops being empty.

No component library and no CSS framework. Styling is hand written, built on custom properties defined in one file of design tokens.

`{@html}` is forbidden across the whole frontend, enforced by a lint rule that fails the build rather than by a convention.

The Tauri API package is a development dependency rather than a runtime one. Everything in a bundled frontend is resolved at build time and inlined into the output; nothing is loaded from `node_modules` when the application runs, so listing it under `dependencies` would describe something that does not happen. It is also first-party code from the framework the application is already built on, so it adds no trust that was not already extended.

## Alternatives considered, and why not

**React.** More people know it, and there is more written about it. Rejected for two reasons. It ships a runtime and a reconciler into the bundle, which is both size and code, and the size matters on the second target platform where the WebView is slower and the device is smaller. And its ecosystem gravity is towards pulling in packages, which is exactly the pressure this decision is trying to resist.

**A component library** such as Material, Bootstrap or a headless toolkit. Rejected because it would be the single largest block of third-party code in the bundle, in an application whose whole purpose is holding secrets. The interface needed here is a few forms, a few lists and a heatmap. That is not worth the dependency, and the accessibility these libraries are usually credited with comes from semantic HTML, which is available without them.

**Tailwind.** Rejected as build tooling that earns its place on a large team keeping many people consistent. There is one person here. Custom properties give the same shared vocabulary of colour and spacing in a file that can be read top to bottom, with no build step to understand and nothing in the dependency graph.

**Allowing `{@html}` with a sanitiser.** Rejected because it trades a rule that cannot be got wrong for one that can. A sanitiser is another dependency, in the most security-sensitive position available, and it only works when somebody remembers to call it. Nothing in this application needs to render markup from data.

**Plain TypeScript with no framework at all.** This was genuinely considered, since it would take the runtime to zero. Rejected because hand written DOM updates are where rendering bugs live, and a rendering bug in a vault is a bug that shows the wrong person's password. Svelte compiles away rather than shipping a runtime interpreter, so the cost is small and what it buys is that the interface is a function of state.

## Consequences

**Good.** The bundle is small: about 37 kB of JavaScript, 14 kB compressed, against a budget of 200 kB. The list of things that could be compromised and reach the vault is short enough to read. The `{@html}` rule closes the realistic cross-site scripting path, and the linter enforces it rather than a reviewer remembering. Styling is one tokens file plus per-component styles, with no build configuration to understand.

**Bad.** Everything an off-the-shelf library would have given has to be written: focus management, dialog behaviour, date handling, the heatmap. Some of that will be worse than a mature library's version at first. Accessibility becomes a deliberate effort on every component instead of something partly inherited. And the empty `dependencies` rule will eventually meet a genuinely hard problem where a well-audited package is the right answer, at which point this document has to be revisited rather than quietly worked around.

**To be revisited when** a runtime dependency is seriously proposed. The reasoning above is the argument it has to beat, and beating it is allowed. What is not allowed is adding one without the argument.
