# 0007 — A window with no system decoration, driven by commands of our own

Date: 2026-09-17 · Status: accepted

## Context

The application draws a header — the brand, the menu of everything that can be opened, the state of the vault with its countdown — and directly under it a strip of tabs. The two are one piece: the tab the eye is on carries a coloured edge that belongs to the same band as the header above it.

With the Windows title bar on top, they are not one piece. They are three stacked bars, the top one belonging to somebody else, in a window whose minimum height is 600 pixels.

A second question arrives with the first. Tauri offers `core:window:allow-start-dragging`, `allow-minimize`, `allow-toggle-maximize` and `allow-close` as capability entries, which is the documented way to drive a window from the interface. This application's capability list currently holds two permissions, both of which can do exactly one thing: subscribe to events the application itself emits, and stop. Every permission in that list is a core API reachable by script that has managed to run inside the WebView, which is the realistic way this application gets compromised.

## Decision

**The window is configured with `decorations: false`.** The application draws its own title bar.

**The four things a title bar does are four commands of our own**, in `src-tauri/src/window.rs`: `start_window_drag`, `minimize_window`, `toggle_maximize_window` and `close_window`. **Not one entry is added to `capabilities/default.json`**, which still grants two permissions and nothing else.

**None of the four takes a parameter.** There is no input from the WebView to validate; the only decision any of them makes is whether the window exists.

**`close_window` closes the vault first and the window second**, and that order is enforced by a function written over two closures so that a test can assert it without a real window.

**The window minimum is 880 by 600**, which is the width the design system stops drawing geometric compositions at and the width the tab cap drops to four at.

**This is brought forward from phase 08**, where the roadmap had it, because the header and the tab strip are being built now and building them under a system title bar means building them twice.

## Alternatives considered, and why not

**Keeping the system title bar until phase 08.** The safe option, and it was the plan. Rejected because the thing being built in this phase is precisely the band that the system title bar sits on top of. Deferring means designing the header twice: once stacked under somebody else's bar, and once not.

**`data-tauri-drag-region` with the four `core:window:*` permissions.** This is the documented route and it is less code. Rejected on surface area. Those four entries are four core APIs that injected script could call, against a list that is currently two entries which can only listen to this application's own events. A custom command needs no capability entry at all, so the version with more code has strictly less attack surface — and the code in question is four functions, each two lines long, over a value that cannot be wrong.

**One command that takes an action name.** `window_control("minimize")` is one command instead of four, and it is the wrong shape: it turns the boundary into a small interpreter with a string from the WebView deciding which branch runs. Four commands that take nothing have no input to validate.

**Closing the window and letting the process exit clear the keys.** Rejected. Closing the window starts tearing down the WebView and the event loop with the key still live, and whatever runs during that teardown runs in a process that is holding a decrypted key with nothing left scheduled that would clear it. Locking first costs nothing and removes the question.

**Re-implementing resize handles in the interface.** Not done, and deliberately: Windows keeps edge and corner resizing for an undecorated window, and re-implementing it would replace something the platform does correctly with something that only looks correct. Whether it also keeps `Win` plus arrow docking and behaves when the window is dragged between monitors with different scaling is the open question this decision rests on, and it is answered by a manual test on a real window rather than by reading.

## Consequences

**Good.** The header and the tab strip are one band, which is what they were designed as.

**Good.** The capability list is unchanged. A phase that added a window feature added nothing an attacker could call.

**Good.** The four commands are the easiest thing in the application to reason about: no parameters, one failure, and the one that matters has its order pinned by a test.

**Bad, and the reason this has its own record.** The application now owns behaviour the platform used to provide. Every one of these has to be checked on a real window rather than in a browser: dragging, double-click to maximise, minimise, restore, resizing from all four edges and all four corners, `Win` plus arrow docking, and dragging to a second monitor with different scaling. If any of them cannot be made to behave, the decision is reverted by reverting one pull request, and the header goes back under the system bar as phase 08 originally planned.

**Bad.** Moving the window is a pointer gesture with no keyboard equivalent, and drawing our own title bar removes whatever the system offered in its place. The window can still be resized by its edges and closed from the keyboard through the control at the end of the bar, but somebody who cannot use a pointer cannot move the window. This is written down rather than discovered: it is a real regression against a system title bar, accepted because the alternative is a header that is two bars.

**Neutral.** The title bar has to appear on the lock screen too, and on every other screen that fills the window. A window that could not be moved or closed until somebody typed a password would be a trap, so the bar is drawn there with no background and only its controls.
