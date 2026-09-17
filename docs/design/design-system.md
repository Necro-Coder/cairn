# Design system

Everything visible in Cairn is built from what is on this page. It is not a style guide to consult when in doubt: it is the source of the values themselves, and a screen that does not come from here is a screen that has to be redone.

Read it before changing anything in `src/`. A colour, a size, a space, a radius or a duration typed by hand into a component is a defect, not a shortcut, and there is a lint rule that fails the build over it.

> **Three choices on this page are proposals, not decisions yet.** They are marked **[proposed]** and are the subject of questions 1, 24, 29 and 30 of the phase questionnaire: the accent colour, the display typeface, and how far the expressive half of the style is allowed to go. Everything else — the structure, the scales, the rules, the enforcement — is settled and does not move with them.

## What the style is

**Expressive minimalism.** The base is quiet to the point of being boring: neutral surfaces, one-pixel borders, no shadows, no gradients, no decoration that carries no information. Against that base, exactly one thing is allowed to be loud, and in Cairn that thing is **type**.

That choice is not a preference, it is the only expressive channel this application can actually afford. Cairn has no network, no runtime dependencies and a strict content security policy, so the fashionable ways to add character — a 3D canvas, an animation library, a downloaded icon set, a web font from a CDN — are either impossible or a new attack surface in an application that holds a password vault. Type costs one bundled file and nothing else.

Everything the interface does is in service of two things, and neither is decoration: knowing at a glance whether the vault is open, and reading data without straining.

### What this rules out, on purpose

The current trend list is full of things that are wrong for this application, and they are written down here so the argument happens once rather than every time somebody sees a nice website.

| Not used | Why |
| --- | --- |
| 3D, WebGL, immersive canvases | A daily tool for reading private data. Weight, battery, attention, and a large amount of code that renders nothing anybody needs |
| Gamification: points, badges, streak confetti | Habits have streaks; the application still does not congratulate anybody. It reports |
| Neumorphism, glassmorphism | Both depend on soft shadows and translucency, which fail at AA contrast and look broken against a light theme |
| Retro-futurism, maximalism, collage, neo-brutalism | All four are identities that compete with the content. This interface is looked at, not admired |
| Scroll-driven storytelling | There is nothing to narrate. There are lists to read |
| Soft drop shadows anywhere | One border weight separates every surface from every other. Two systems of elevation is one too many |

What is kept from the same list: **bold type**, **dark mode** (already the default), **restrained micro-animation**, and **sustainable design** read as what it actually means here — a small bundle, no third-party code, and an interface that works for somebody navigating with a keyboard.

## Colour

Colour carries meaning or it is not used. Every surface is neutral; the one accent marks the single most important action on a screen; the four status colours never appear without a word next to them, because a person who cannot distinguish red from green still has to be able to read the screen.

### Roles

| Token | Role |
| --- | --- |
| `--colour-surface` | The window itself |
| `--colour-surface-raised` | Anything that sits on it: cards, panels, the sidebar |
| `--colour-surface-sunken` | Anything recessed: inputs, code, the empty part of a chart |
| `--colour-border` | Ordinary separation between surfaces |
| `--colour-border-strong` | Separation that has to be noticed: a focused input, a selected row |
| `--colour-text` | What you came to read |
| `--colour-text-muted` | Labels, help, secondary information |
| `--colour-text-faint` | Timestamps, counts, anything you read only when looking for it |
| `--colour-accent` | The one important action on the screen, and the focus ring |
| `--colour-accent-strong` | That action under the pointer |
| `--colour-accent-contrast` | Text on top of the accent |
| `--colour-positive` `--colour-warning` `--colour-negative` `--colour-info` | State, always beside a word |

### Values

Dark is the default and light is a media query, because dark is what the application is used in. Neither theme is an afterthought: every screen is looked at in both before it is finished.

```css
:root {
  color-scheme: dark light;

  --colour-surface: #14161a;
  --colour-surface-raised: #1c1f25;
  --colour-surface-sunken: #0f1013;
  --colour-border: #2a2f38;
  --colour-border-strong: #3c4350;

  --colour-text: #e8eaed;
  --colour-text-muted: #a2a9b5;
  --colour-text-faint: #6f7683;

  /* [proposed] A warm accent on a cool neutral base: it reads as deliberate rather than
     as the default blue every framework ships, and it stays clear of the status colours. */
  --colour-accent: #e0a458;
  --colour-accent-strong: #f2bb76;
  --colour-accent-contrast: #14161a;

  --colour-positive: #7fca88;
  --colour-warning: #e0b978;
  --colour-negative: #f2777a;
  --colour-info: #7aa2f7;
}
```

### Rules

- **Contrast is AA everywhere and AAA on the two screens that are read in a hurry**: the unlock screen and anything showing a password. 4.5:1 for text, 3:1 for interface elements and focus rings.
- **The accent is rationed.** One per screen. A screen with two accented buttons has not decided what it is for.
- **Status colour is never the only signal.** Red plus the word, not red alone.
- **No colour literal outside this file.** Not in a component, not in a `<style>` block, not "just this once for a border".

## Type

Type is where the character lives, so it is the part of this document with the most rules.

### The families

| Token | Used for | Source |
| --- | --- | --- |
| `--font-display` | Screen titles, and nothing else | **[proposed]** one bundled variable font, subset to Latin, served from the bundle |
| `--font-sans` | Everything else | The system stack. Segoe UI Variable on Windows, San Francisco on iOS |
| `--font-mono` | Amounts, identifiers, key fingerprints, diagnostics | The system mono stack |

A bundled face is an asset, not a dependency: it ships as a file in `src/lib/fonts/`, it is referenced by a local `@font-face`, and the runtime dependency count stays at zero. It must be subset, must be a variable font, must be under 80 KB, and must declare a real fallback in the same stack, so a build that cannot load it still renders a sensible page.

Nothing is ever loaded from a font CDN. The content security policy blocks it, and that is the correct behaviour rather than an obstacle to work around.

### The scale

A modular scale at a ratio of 1.25 anchored on a 15px body. The display sizes jump past the ratio on purpose: that jump is the expressive part.

| Token | Size | Used for |
| --- | --- | --- |
| `--text-xs` | 0.75rem / 12px | Timestamps, counters |
| `--text-sm` | 0.8125rem / 13px | Labels, help text |
| `--text-base` | 0.9375rem / 15px | Body, lists, inputs |
| `--text-lg` | 1.1875rem / 19px | Section subheadings |
| `--text-xl` | 1.5rem / 24px | Card titles |
| `--text-2xl` | 1.875rem / 30px | Screen titles |
| `--text-3xl` | 2.375rem / 38px | The unlock screen, and the one title per screen that is allowed to be large |
| `--text-4xl` | 2.9375rem / 47px | Reserved. One use per application, if any |

- Line height: `--leading-tight` (1.25) for anything above `--text-lg`, `--leading-normal` (1.55) for body.
- Weight: 400 for body, 500 for labels and buttons, 600 for headings. Nothing heavier, and nothing lighter than 400 at body size.
- Letter spacing: slightly negative on display sizes, never on body.
- Measure: prose stays within `--measure` (68 characters). Lists and tables are allowed the full width.
- **Never** use size alone to convey importance where a heading element would do it properly: the hierarchy has to survive being read aloud by a screen reader.

## Space and shape

- **Space** is a four pixel grid: 4, 8, 12, 16, 24, 32, 48, 64, as `--space-1` to `--space-8`. Nothing in between, ever.
- **Radius** is `--radius-sm` 4px for inputs and small controls, `--radius-md` 8px for cards and panels, `--radius-lg` 12px for dialogs. Nothing is fully rounded except an avatar, and there are no avatars.
- **Borders** are one pixel, `--border-width`. This is the only way surfaces are separated.
- **Elevation does not exist.** There is no shadow scale. A panel above the page is a panel with a border and a raised surface colour. A dialog additionally dims what is behind it.
- **Density** is single and comfortable. A compact mode is a decision for the day there are long lists to justify it.

## Motion

Motion exists to explain a change of state, never to decorate one.

- `--duration-fast` 90ms for anything under the pointer: hover, press, focus.
- `--duration-normal` 160ms for something appearing or disappearing.
- Nothing is slower than 250ms. This application is opened fifty times a day and every millisecond above that reads as lag.
- Only `transform` and `opacity` are animated. Animating width, height or colour on a list is how a fast interface becomes a slow one.
- `prefers-reduced-motion: reduce` sets every duration to zero. It is set once, globally, in `tokens.css`, so nobody has to remember it per component.

## Icons

Icons are hand-written inline SVG in Svelte components under `src/lib/icons/`. There is no icon package, because an icon package is a runtime dependency and the dependency count is a gate.

- 24px box, 1.5px stroke, round caps, no fill.
- `currentColor` always, so an icon inherits the colour of the thing it belongs to.
- Every icon that is not purely decorative carries an accessible name; every icon that is decorative is hidden from assistive technology.
- An icon never appears alone on a control whose meaning is not already obvious from context. When in doubt, a word.

## Components

These are rules, not a catalogue. The catalogue is the code.

**Buttons.** One accented button per screen at most. Everything else is a bordered button on a raised surface, or plain text with an underline on hover. A disabled button explains why it is disabled, through its `title` and, where it matters, through visible text: a control that is dead for an unexplained reason is a bug report waiting to be filed.

**Inputs.** Label above, always a real `<label>`, never a placeholder standing in for one. Help text below in `--colour-text-muted`. Errors below that, in `--colour-negative`, announced with `role="alert"`. A sunken surface, a one pixel border, and `--colour-border-strong` plus the focus ring when focused.

**Cards and panels.** Raised surface, one pixel border, `--radius-md`, `--space-5` of padding. A title in `--text-xl`. No shadow.

**Lists.** Rows separated by a one pixel border, not by alternating background. Hover raises the surface, it does not tint it with the accent. A selected row is marked with `--colour-border-strong` and a left edge in the accent.

**Empty states.** Every list has one, and it is the real thing the person will see on their first day rather than a placeholder: a large faint icon, one sentence saying what will be here, and the action that puts something there. Never the word "empty" alone.

**Warnings that matter.** The ones about losing data are the exception to every rule about restraint on this page: they go before the fields rather than after, they use `--colour-warning` on the border and the title, and they say the whole thing in plain Spanish. A warning under a form is a warning read after the decision.

**Dialogs.** Only for a choice that cannot be undone by carrying on. The rest is done in place. A dialog traps focus, closes on `Escape`, returns focus where it came from, and never appears without the person having asked for something.

## Layout

- A fixed sidebar on the left with the three modules and settings, a header with the current section and the state of the vault, and the content to the right.
- Content is capped at 1100px and centred inside whatever is left.
- Below 880px wide, the sidebar collapses to icons. Below 420px it becomes a bottom bar; this is the phone shape, and phase 09 is what makes it good rather than merely unbroken.
- The unlock screen has no shell at all. It is the boundary between being in and being out, and it should not look like a page of the application with a form on it.
- Safe area insets are respected on every edge, because the same markup runs inside a phone.

## Writing

The interface is in Spanish. The code, the identifiers and this document are in English.

- Plain words. "Se cerrará sola en 30 segundos", not "Sesión finalizada por inactividad".
- Long and explicit where there is risk: creating a vault, changing the master password, deleting something. Telegraphic everywhere else.
- Errors say what happened and what to do, and never blame the person.
- No exclamation marks, no emoji, no congratulation. The application reports; it does not celebrate.
- The vault is "la caja fuerte" everywhere. One word per concept, across the whole interface.

## Accessibility

WCAG 2.2 AA is the floor and it is not negotiable, because the alternative is an application that stops working the day its author has a bad wrist.

- Everything usable with a mouse is usable with a keyboard, in a logical order, with the focus ring always visible. The ring is defined once, in `base.css`, and is never removed.
- Semantic HTML first. A `div` with an `onClick` is a defect.
- Every control has an accessible name. Every state change that is not visible where the eye already is gets an `aria-live` region.
- Contrast is verified, not estimated.
- Touch targets are at least 24 by 24 pixels.
- The screen is checked once with `Tab` alone before the work is called done.

## How this is kept true

A design system that lives only in a document is a document. Three things keep this one in the code.

1. **Tokens are the only source of values.** `src/lib/styles/tokens.css` holds them and nothing else defines one. A component that needs a value that does not exist adds it here, with a comment saying why, rather than typing a number.
2. **A lint rule fails the build** on a colour literal, an off-scale length or a hand-written duration inside `src/`. Erosion happens one reasonable exception at a time, and a gate is what makes each exception argue for itself.
3. **The review question is fixed**: does every value in this diff come from a token, does the screen work in both themes, does it work with the keyboard alone, and does it work with motion reduced. Four questions, asked every time.
