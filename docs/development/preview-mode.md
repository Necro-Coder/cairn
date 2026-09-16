# Preview mode

A way to open the interface in an ordinary browser, with invented data behind it.

```
npm run preview:mock
```

That starts Vite with the core replaced by a module that makes its answers up, and prints a `http://localhost` address. Open it in any browser. Both screens work, the keyboard shortcut works, and a banner across the top says, without any way to dismiss it, that nothing you are looking at is real.

## What it is for

Looking at the interface somewhere the desktop window cannot go.

The obvious case is a phone. The application is eventually meant to run inside WKWebView on iOS, and Safari on an iPhone is the same engine. Opening this address from the phone, over a private network, shows how the layout behaves on a real small screen with a real mobile WebView, months before there is anything to install on it.

The less obvious case is simply being able to look at two versions of a screen side by side, in tabs, with the browser's own developer tools, without restarting a desktop application.

## What it is not for

**It validates nothing about the core.** There is no Rust behind a browser tab. No encryption happens, no database is opened, no command crosses any boundary. A screen that looks right here can still be broken in the real application, and the reverse is also true.

**Its numbers are invented.** The diagnostics screen will show a start-up time and a command latency. Those are values typed into a source file. Reading a performance budget off a preview is reading somebody's guess with extra steps.

**It is not a test double.** Tests run against the real boundary, or against Rust. Nothing in the test suite imports this module, and nothing should.

The honest summary is that it proves the interface renders. That is all it claims, and it is worth having for exactly that.

## How it is kept out of the real application

An application that holds a password vault must never show invented data as though it were the person's own. Two things stop it, and they work at different levels on purpose.

**The alias.** Every file in the frontend imports `$ipc` and never a concrete path. `vite.config.ts` decides what is behind that name: the real boundary normally, the stand-in when the mode is `preview`. So in a normal build the module that invents data is not in the module graph at all. It is not disabled by a condition somebody could get wrong; there is nothing there to reach.

**The marker.** The stand-in defines one string, `CAIRN-PREVIEW-MOCK-DATA`, and that same string is what the banner puts on screen. After every production build the pipeline searches the whole of `dist/` for it and fails if it is found.

The second exists because the first is a statement of intent and the check has to be about the artefact. A single mistaken import would undo the alias without producing any warning. And because the marker is one string with both jobs, the thing the pipeline looks for and the thing a person sees cannot drift apart.

Both were verified by doing it the wrong way round: building in preview mode and confirming the check finds the marker, then building normally and confirming it does not.

A preview build also writes to `dist-preview/` rather than `dist/`, so an absent-minded `npm run build:preview` cannot leave a bundle full of invented data sitting where the packaging step looks for the real one.

## Why the two implementations cannot drift apart

`src/lib/ipc.types.ts` declares `IpcSurface`: every function the interface may call, with its signature. Both implementations are annotated with it.

Add a command to the real boundary and forget the stand-in, and the stand-in fails to compile because it is missing a member. Add one to the stand-in that does not exist on the real side, and it fails because `IpcSurface` does not declare it. Either way `npm run check` stops, which is the point: a written convention that the two should match would hold until the first time somebody was in a hurry.

The `previewNotice` on that type is what the banner reads. The real boundary answers `null`, so the warning text is not hidden behind a condition in a production build, it simply does not exist there. The module that invents the data is the one that declares it did.

## Opening it from a phone

By default the server listens only on the machine it runs on, which is the right default: this is a development server, it has no authentication, and anything it is reachable from can talk to it.

To reach it from another device on a private network:

```
npm run preview:mock -- --host
```

That binds to every interface on the machine. Use it on a network you control, for as long as you need it, and stop it afterwards. Nothing here is published anywhere, and the preview is deliberately not deployed to any hosting: a page that looks like the interface of an application holding a password vault is worth nothing to anyone and is worth something to somebody building a convincing imitation of it.

A dev server bound to a network serves files, not just the page. By default Vite will hand out anything inside the project directory to anybody who asks for it by name, and a working directory holds more than the files git tracks: notes, drafts, whatever was left there while working. So `server.fs.allow` in `vite.config.ts` lists what may be read — the entry page, the frontend sources, the isolation application and the installed packages — and everything else answers 403.

An allowlist rather than a list of things to block. A denylist only ever covers the files somebody already thought of, and the one that matters is the one nobody thought of.

This was checked by asking for those files over the network before the allowlist existed and getting them, then again afterwards and getting 403.

## The banner

It is sticky, it takes its own row rather than covering the first one, and there is no way to close it.

That last part is deliberate and is the whole reason it exists. The failure this guards against is not somebody being unable to tell; it is somebody who knew twenty minutes ago, has been reading the screen since, and makes a decision based on a number this file invented. A warning that can be dismissed is one that will be.
