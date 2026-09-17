# 0006 — Tabbed navigation, and typefaces bundled as assets

Date: 2026-09-17 · Status: accepted

## Context

Two decisions are recorded together because they are the same kind of decision: both are cheap to make once and expensive to reverse after fifteen screens have been written against them.

The first is the shape of navigation. Until now the application had four screens and a router that picked between them with a chain of conditions, which is what four screens deserve. It is about to have fifteen, across three modules plus settings, and something has to decide how somebody moves between them and what they see when they arrive.

The second is where the type comes from. The interface is meant to have a visual identity built on the extreme pairing of very large display type against very small tracked-out labels, which is what the Swiss poster references it is drawn from actually do. A system font stack cannot do that: it has no width control, no reliable variable weight axis, and it looks like a different application on each of the two target platforms. The constraint is that Cairn has zero runtime dependencies and a content security policy with no `font-src` of its own, so a font CDN is refused by the application's own defences.

## Decision

**Navigation is a header, a tab strip and content, in that order down the window.** There is no sidebar. The header holds the `Abrir` menu, the state of the vault and the window controls; the strip below it holds the tabs.

**Everything opened is opened in a tab, and every tab is born temporary.** A temporary tab is drawn in italics with a dotted underline and occupies one fixed slot, so the next thing opened replaces it rather than accumulating beside it. It becomes permanent on a double click, or when something inside it is edited. Reopening the last closed tab is the single exception: it comes back permanent, because it was asked for deliberately.

**The tab strip never scrolls.** The cap is six tabs plus the panel, and four below 880 pixels of window width. Past the cap, opening something recycles the temporary slot instead of shrinking what is already there.

**The panel is the first tab and cannot be closed.** It is what appears on unlocking, it is composed by the person out of cards each module offers, and it starts empty.

**Navigation state is treated as sensitive.** Tabs and the session history are discarded the moment the vault closes, the history is never written anywhere at all, and when tabs are eventually remembered between runs they will be kept inside the encrypted database rather than in a settings file.

**Archivo and JetBrains Mono are bundled as assets**: two `woff2` files in `src/lib/fonts/`, Latin subset, variable, weight axis only, declared with a local `@font-face`, each with its `OFL.txt` beside it. They are not npm packages, so the runtime dependency count stays at zero and its gate stays green.

**Archivo ships without its width axis.** With both axes the Latin subset is 90 KB; with the weight axis alone it is 35 KB. The budget was 80 KB, and the budget won.

## Alternatives considered, and why not

**A fixed sidebar, which is what the design system originally said.** It is the obvious shape for three modules and settings, and it is what the first draft of the design document described. It was rejected once the navigation model was decided, for a reason that only appears when the two are put together: the sidebar answers "which of four places am I in", and tabs answer "which of the things I have open am I looking at". With tabs, a sidebar is a second, weaker copy of the tab strip taking a permanent column of a window whose minimum is 880 pixels wide.

**Tabs that accumulate, as a browser does.** Rejected because a browser's twentieth tab is eight pixels wide and unreadable, and the failure is silent: nothing tells you the strip has stopped being useful. A hard cap with a recycled temporary slot means the strip always reads, at the cost of occasionally reusing a slot somebody wanted to keep — which a double click prevents, and which is recoverable.

**Everything opens permanent, with a separate "preview" gesture.** This is the inverse default, and it is what makes a browser accumulate. Making temporary the default and permanent the deliberate act is the only arrangement where the common case — glancing at something — costs nothing to clean up.

**Split view, two tabs side by side.** Deferred rather than rejected. It is the natural next step and it is a large amount of work, and there is no data yet to look at in parallel. Revisiting it later costs nothing that has been built here.

**Remembering tabs between runs, now.** Rejected for this phase, and the reason is the second decision above: a tab called "Visa · Banco" written into an ordinary settings file is a list of the person's accounts sitting in plain text on the disk. It waits for the encrypted database.

**A system font stack, which is what the project shipped until now.** Zero bytes, renders natively, and cannot produce the one thing the identity rests on. It remains the fallback in every stack, so a build that cannot load a face still renders a sensible page.

**A font from Google Fonts or any other CDN.** Refused by the content security policy, which is correct behaviour rather than an obstacle. It would also be a request to a third party every time the application starts, in an application whose whole premise is that it talks to nobody.

**An npm package that wraps a typeface.** Rejected. It is the same bytes with a dependency wrapped around them, and the dependency count is a gate for a reason: every package in the graph is a supply chain risk in an application holding a password vault.

**Keeping Archivo's width axis and raising the budget to 90 KB.** Rejected, and this is the one worth recording because the earlier draft of the design system asserted the width axis "costs nothing extra in a variable font". Measured, it costs 55 KB, which is more than double the file. The width axis is a refinement of how a title is composed; the identity is the size pairing, the vermilion and the flat geometry. A budget that is raised the first time it is inconvenient is not a budget.

**Bundling a Nerd Font for the monospace, which was asked for.** Rejected on measurement. A Nerd Font is the base family plus roughly ten thousand icon glyphs, between 2 and 6 MB per variant against a 60 KB budget, and its icon glyphs would collide with the eighteen hand-written icons this system defines. The base family is what is actually read in an amount, an identifier or a diagnostics value, and that is what ships.

## Consequences

**Good.** The header and the tab strip are one piece rather than two stacked bars, which is what made the undecorated window worth doing at the same time — that is recorded separately, in [decision 0007](0007-undecorated-window.md).

**Good.** The temporary tab makes the common case free. Glancing at four things in a row leaves one tab behind rather than four.

**Good.** Discarding tabs on lock is one assignment in one place, next to where everything else derived from an open vault is already discarded. It costs nothing and closes a real leak: the strip is the part of the window a person walking past actually reads.

**Good.** Two bundled faces cost 66 KB together, against a combined budget of 140 KB, and both are subset so a character outside Latin comes from the fallback rather than forcing a download that would not have the glyph.

**Bad.** A tab strip with a cap will occasionally recycle a slot somebody meant to keep. The double click that prevents it has to be learned, which is why the strip says so, in the place where the count would otherwise be, whenever the active tab is temporary.

**Bad.** Tabs do not survive a lock or a restart, and that is a real inconvenience roughly twice a day. It is the price of the decision above, taken deliberately.

**Bad.** Archivo without its width axis means a display title is composed by size, weight and letter-spacing alone. It is a slightly narrower instrument than the design intended.

**Neutral.** Bundled type is two files somebody has to remember to update if the upstream project fixes a glyph. Nothing does that automatically, and nothing should: a typeface that changed shape without anybody deciding would change every screen.
