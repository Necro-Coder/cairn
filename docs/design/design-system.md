# Design system

Everything visible in Cairn is built from what is on this page. It is not a style guide to consult when in doubt: it is the source of the values themselves, and a screen that does not come from here is a screen that has to be redone.

Read it before changing anything in `src/`. A colour, a size, a space, a radius or a duration typed by hand into a component is a defect, not a shortcut, and there is a lint rule that fails the build over it.

## Where the style comes from

The reference is Swiss poster design and the Bauhaus: flat colour, hard edges, a visible grid, geometric marks made of circles and thick rules, one or two saturated colours against paper or near-black, and type that is either very large or very small with nothing much in between.

That language was made for posters, and Cairn is a tool for reading private data every day, so it is translated rather than copied. What comes across:

| Taken | How it appears in an application |
| --- | --- |
| Flat colour, no shadow, no gradient | One border weight separates every surface. There is no elevation scale at all |
| Hard edges | Radii of 2 and 4 pixels. Nothing looks soft, nothing is a pill |
| A saturated accent used with discipline | One vermilion accent, one action per screen. Blue and yellow carry meaning, not decoration |
| Paper and ink | The light theme is warm paper, not grey. The dark theme is near-black, not blue-grey |
| Enormous type against tiny type | A display scale for titles and a tracked-out uppercase label style. The gap between them is the composition |
| Geometric marks | Circles, half circles and thick rules, composed like a poster on the screens that hold no data, and absent from every screen that does |
| The grid, and deliberate asymmetry | Content sits on a real grid, and the title block is allowed to break it |

What does not come across, because a poster is looked at once and an application is looked at a thousand times: full-bleed blocks of saturated colour, colour as the background of anything containing data, decorative shapes anywhere near a number, and compositions that need to be studied.

## What the style is

**Expressive minimalism.** The base is quiet to the point of being boring: paper or ink, one-pixel borders, no shadows, no gradients, no decoration that carries no information. Against that base, three things are allowed to be loud: **type**, **one vermilion accent**, and **flat geometry** — and the third one is allowed only on the screens that have no data on them.

That last division is the whole discipline of this system. A screen where somebody reads their own passwords or their own money gets type and one accent and nothing else. A screen where there is nothing to read yet — the lock screen, the first run, an empty list — is where the poster is allowed to happen.

That restraint is not only taste. Cairn has no network, no runtime dependencies and a strict content security policy, so the fashionable ways to add character — a 3D canvas, an animation library, a downloaded icon set, a web font from a CDN — are either impossible or new attack surface in an application that holds a password vault. Type and flat colour cost one bundled file and nothing else.

### What this rules out, on purpose

Written down so the argument happens once rather than every time somebody sees a nice website.

| Not used | Why |
| --- | --- |
| 3D, WebGL, immersive canvases | Weight, battery, attention, and a large amount of code that renders nothing anybody needs |
| Gamification: points, badges, streak confetti | Habits have streaks; the application still does not congratulate anybody. It reports |
| Neumorphism, glassmorphism | Both depend on soft shadows and translucency, which fail at AA contrast and contradict every reference on this page |
| Retro-futurism, maximalism, collage, neo-brutalism | Identities that compete with the content. This interface is looked at, not admired |
| Scroll-driven storytelling | There is nothing to narrate. There are lists to read |
| Drop shadows, anywhere | One border weight separates every surface from every other. Two systems of elevation is one too many |

Kept from the current trend list: **bold type**, **dark mode**, **restrained micro-animation**, and **sustainable design** read as what it actually means here — a small bundle, no third-party code, and an interface that works for somebody navigating with a keyboard.

## Colour

Colour carries meaning or it is not used. Surfaces are paper or ink; the accent marks the single most important action on a screen; the status colours never appear without a word beside them, because somebody who cannot tell red from green still has to be able to read the screen.

The Bauhaus triad maps onto the three things this application has to say, which is a happy accident worth keeping: **vermilion** is the accent and means "this is the action", **blue** means information, **yellow** means caution.

### Roles

| Token | Role |
| --- | --- |
| `--colour-surface` | The window itself |
| `--colour-surface-raised` | Anything that sits on it: cards, panels, the sidebar |
| `--colour-surface-sunken` | Anything recessed: inputs, the empty part of a chart |
| `--colour-border` | Ordinary separation between surfaces |
| `--colour-border-strong` | Separation that has to be noticed: a focused input, a selected row |
| `--colour-rule` | The thick rule under a section title. Ink, not accent |
| `--colour-text` | What you came to read |
| `--colour-text-muted` | Labels, help, secondary information |
| `--colour-text-faint` | Timestamps, counts, anything read only when looked for |
| `--colour-accent` | The one important action on the screen, and the focus ring |
| `--colour-accent-strong` | That action under the pointer |
| `--colour-accent-contrast` | Text on top of the accent |
| `--colour-positive` `--colour-warning` `--colour-negative` `--colour-info` | State, always beside a word, always safe as text |
| `--mark-vermilion` `--mark-blue` `--mark-yellow` `--mark-ink` | The poster palette, for geometric marks only. Never text, never behind data |

The split between status colours and mark colours is deliberate. Full-saturation yellow is unreadable as text on paper and perfect as a filled circle; keeping them as separate tokens is what stops somebody solving a contrast failure by desaturating a mark, or brightening a status colour until it stops being legible.

### Values

```css
:root {
  color-scheme: dark light;

  /* Ink. Near-black and neutral, so the accent is the only warm thing on screen. */
  --colour-surface: #101011;
  --colour-surface-raised: #191a1c;
  --colour-surface-sunken: #0a0a0b;
  --colour-border: #2a2b2e;
  --colour-border-strong: #3e4044;
  --colour-rule: #f2efe9;

  /* Paper-white text, slightly warm, so a dark screen full of text does not read as blue. */
  --colour-text: #f2efe9;
  --colour-text-muted: #a5a29b;
  --colour-text-faint: #6e6c67;

  --colour-accent: #ff5a1f;
  --colour-accent-strong: #ff7a45;
  --colour-accent-contrast: #101011;

  --colour-positive: #63c281;
  --colour-warning: #e8b931;
  --colour-negative: #f2696d;
  --colour-info: #5b8cff;

  --mark-vermilion: #ff5a1f;
  --mark-blue: #1b4fd8;
  --mark-yellow: #f2c007;
  --mark-ink: #f2efe9;
}

@media (prefers-color-scheme: light) {
  :root {
    /* Paper. Warm and slightly off, like the references, never pure white. */
    --colour-surface: #f4f1ea;
    --colour-surface-raised: #fbf9f5;
    --colour-surface-sunken: #e9e5db;
    --colour-border: #d9d3c6;
    --colour-border-strong: #b3ab9b;
    --colour-rule: #14130f;

    --colour-text: #14130f;
    --colour-text-muted: #55514a;
    --colour-text-faint: #7c776e;

    --colour-accent: #dd3f0c;
    --colour-accent-strong: #b93307;
    --colour-accent-contrast: #fbf9f5;

    --colour-positive: #2c7a44;
    --colour-warning: #8a5d10;
    --colour-negative: #b8272c;
    --colour-info: #2450c8;

    --mark-vermilion: #ff5a1f;
    --mark-blue: #1b4fd8;
    --mark-yellow: #f2c007;
    --mark-ink: #14130f;
  }
}
```

### Rules

- **Contrast is AA everywhere, AAA on the two screens read in a hurry**: the unlock screen and anything showing a password. 4.5:1 for text, 3:1 for interface elements and focus rings.
- **One accent per screen.** A screen with two accented buttons has not decided what it is for.
- **Status colour is never the only signal.** Red plus the word, not red alone.
- **Mark colours never carry meaning.** If removing the shape would lose information, it was not a mark.
- **No colour literal outside `tokens.css`.** Not in a component, not in a `<style>` block, not "just this once for a border".

## Type

Type is where most of the character lives, so this is the section with the most rules.

### The families

| Token | Used for | Source |
| --- | --- | --- |
| `--font-display` | Screen titles, section names, and numbers that are the point of the screen | **Archivo**, variable, bundled, Latin subset. Decided |
| `--font-sans` | Everything else | The system stack: Segoe UI Variable on Windows, San Francisco on iOS |
| `--font-mono` | Amounts, identifiers, key fingerprints, diagnostics | The system mono stack |

Archivo is a grotesque in the Swiss line with a weight axis and a width axis, which is exactly what the references do with type: very large, very tight, very plain. The width axis is what makes a title look composed rather than merely big, and it costs nothing extra in a variable font.

A bundled face is an asset, not a dependency: it ships as a file in `src/lib/fonts/`, is referenced by a local `@font-face`, and the runtime dependency count stays at zero. It must be subset to Latin, must be variable, must stay under 80 KB, and must declare a real fallback in the same stack, so a build that cannot load it still renders a sensible page.

Nothing is ever loaded from a font CDN. The content security policy blocks it, and that is correct behaviour rather than an obstacle to work around.

### The scale

A modular scale at a ratio of 1.25 anchored on a 15px body. The display sizes jump past the ratio on purpose: that jump is the composition.

| Token          | Size             | Used for                                     |
| -------------- | ---------------- | -------------------------------------------- |
| `--text-label` | 0.6875rem / 11px | Uppercase tracked labels. See below          |
| `--text-xs`    | 0.75rem / 12px   | Timestamps, counters                         |
| `--text-sm`    | 0.8125rem / 13px | Help text, secondary rows                    |
| `--text-base`  | 0.9375rem / 15px | Body, lists, inputs                          |
| `--text-lg`    | 1.1875rem / 19px | Section subheadings                          |
| `--text-xl`    | 1.5rem / 24px    | Card titles                                  |
| `--text-2xl`   | 1.875rem / 30px  | Screen titles                                |
| `--text-3xl`   | 2.625rem / 42px  | The one title per screen allowed to be large |
| `--text-4xl`   | 3.75rem / 60px   | The lock screen, and nothing else            |

- **The label style is the signature of this system.** `--text-label`, uppercase, weight 600, `letter-spacing: 0.09em`, in `--colour-text-muted`. It names a section, a field group or a column, and it is the small half of the big-and-small pairing the references are built on.
- Line height: `--leading-tight` (1.1) at `--text-2xl` and above, `--leading-snug` (1.3) between, `--leading-normal` (1.55) for body.
- Letter spacing: `-0.02em` on display sizes, `-0.01em` at `--text-xl`, none on body, `0.09em` on labels.
- Weight: 400 body, 500 labels and buttons, 600 headings. Display titles use 600 with the width axis at 100 or slightly expanded. Nothing lighter than 400 at body size.
- Measure: prose stays within `--measure` (68 characters). Lists and tables take the full width.
- **Never** use size alone where a heading element would do the job properly: the hierarchy has to survive being read aloud.

## Space and shape

- **Space** is a four pixel grid: 4, 8, 12, 16, 24, 32, 48, 64, as `--space-1` to `--space-8`. Nothing in between, ever.
- **Radius** is sharp: `--radius-sm` 2px for inputs and buttons, `--radius-md` 4px for cards and panels, `--radius-lg` 8px for dialogs. Nothing is fully rounded. There are no pills.
- **Borders** are one pixel, `--border-width`, and are the only way surfaces are separated.
- **The rule** is the thick line the references use to anchor a title block: `--rule-width` 3px in `--colour-rule`, under a screen title and nowhere else. It is the one piece of pure typographic furniture in the system.
- **Elevation does not exist.** No shadow scale. A panel above the page is a raised surface with a border. A dialog additionally dims what is behind it.
- **Density** is single and comfortable. A compact mode is a decision for the day there are long lists to justify it.

## Geometric marks

Circles, half circles, thick rules and the occasional square, flat and from the mark palette. They are the part of the references that gives the application a face, and they are also the part that can ruin it, so the rule is not "use them tastefully": it is a list of places, a budget, and four prohibitions that hold everywhere.

### Where they are allowed

| Place | Budget | What it looks like |
| --- | --- | --- |
| The lock screen | Up to four shapes | The full composition. A large vermilion circle breaking the top right corner, a blue half circle sitting on its lower edge, an ink rule crossing the width, one small yellow dot low on the opposite side. Fixed: the same arrangement every time, because this screen is seen twice a day and a composition that moves is a composition that irritates |
| Creating a vault, and the damaged-header screen | Up to two shapes | A reduced version of the same idea. They are the other two screens with no data on them |
| An empty state | Up to three shapes | Its own small composition, no more than 160px across, above the sentence |
| A module section header | Exactly one | A small filled circle before the label, in that module's colour: blue for habits, yellow for finances, ink for passwords |
| The bottom of the sidebar | Exactly one | A small ink mark, the application's own sign |

### Where they are never allowed

Anywhere with data on it. A list, a table, a card showing a value, a form, a dialog, a chart, a row, a total. No exceptions, and not "faintly in the background either": a number is read, and anything behind it competes for the same attention.

### The four prohibitions

1. **Nothing is ever placed on top of a mark, and a mark is never placed on top of anything.** Geometry lives in its own region of the layout. Where the window is too narrow for that region, the mark is removed, not shrunk into the content: below 880px the composition drops to one shape, below 420px to none.
2. **A mark is never animated**, never on load, never on hover, never on the way in.
3. **A mark never responds to the pointer.** `pointer-events: none`, always.
4. **A mark never means anything.** It is `aria-hidden`, it is hidden in forced-colors mode, and a screen that reads worse without it was never decorated — it was under-labelled, and the fix is a word.

Flat fill only, no stroke unless the shape is itself a rule, no gradient, no transparency, no overlap that produces a third colour. If somebody has to look twice to work out whether a shape means something, the shape is wrong.

## Motion

Motion explains a change of state; it never decorates one.

- `--duration-fast` 90ms for anything under the pointer: hover, press, focus.
- `--duration-normal` 160ms for something appearing or disappearing.
- Nothing is slower than 250ms. This application is opened fifty times a day and anything above that reads as lag.
- Only `transform` and `opacity` are animated. Animating width, height or colour across a list is how a fast interface becomes a slow one.
- `prefers-reduced-motion: reduce` sets every duration to zero, once, globally, in `tokens.css`, so nobody has to remember it per component.

## Icons

Icons are hand-written inline SVG in Svelte components under `src/lib/icons/`. There is no icon package, because an icon package is a runtime dependency and the dependency count is a gate.

- 24px box, 1.5px stroke, round caps, no fill, geometric construction to match the type.
- `currentColor` always, so an icon inherits the colour of what it belongs to.
- Every icon that is not decorative carries an accessible name; every decorative one is hidden from assistive technology.
- An icon never stands alone on a control whose meaning is not obvious from context. When in doubt, a word.

## Components

Rules, not a catalogue. The catalogue is the code.

**Buttons.** At most one accented button per screen: solid vermilion, `--colour-accent-contrast` text, 2px radius. Everything else is a bordered button on a raised surface, or plain text underlined on hover. A disabled button explains why, through its `title` and, where it matters, in visible text: a control that is dead for an unexplained reason is a bug report waiting to be filed.

**Inputs.** Label above in the label style, always a real `<label>`, never a placeholder standing in for one. Help below in `--colour-text-muted`, errors below that in `--colour-negative` with `role="alert"`. Sunken surface, one pixel border, `--colour-border-strong` plus the focus ring when focused.

**Cards and panels.** Raised surface, one pixel border, `--radius-md`, `--space-5` of padding, title in `--text-xl`. No shadow.

**Lists.** Rows separated by a one pixel border, never by alternating background. Hover raises the surface rather than tinting it. A selected row carries `--colour-border-strong` and a 3px accent edge on the left.

**Empty states.** Every list has one, and it is the real thing the person sees on their first day rather than a placeholder: a geometric mark, one sentence saying what will be here, and the action that puts something there. Never the word "empty" on its own.

**Warnings that matter.** The ones about losing data are the exception to every rule about restraint on this page: they go before the fields rather than after, they carry `--colour-warning` on the border and the title, and they say the whole thing in plain Spanish. A warning under a form is a warning read after the decision.

**Dialogs.** Only for a choice that carrying on cannot undo. Everything else happens in place. A dialog traps focus, closes on `Escape`, returns focus where it came from, and never appears unasked.

## Layout

- A fixed sidebar on the left with the three modules and settings, a header with the current section and the state of the vault, content to the right.
- Content is capped at 1100px and centred in what is left. A screen title sits in a block of its own above the content, with the rule under it.
- Below 880px the sidebar collapses to icons. Below 420px it becomes a bottom bar; that is the phone shape, and phase 09 is what makes it good rather than merely unbroken.
- The unlock screen has no shell. It is the boundary between being in and being out, it fills the window, and it is the one screen built like a poster: the mark, the name at display size, and one field.
- Safe area insets are respected on every edge, because the same markup runs inside a phone.

## Writing

The interface is in Spanish. The code, the identifiers and this document are in English.

- Plain words. "Se cerrará sola en 30 segundos", not "Sesión finalizada por inactividad".
- Long and explicit where there is risk: creating a vault, changing the master password, deleting. Telegraphic everywhere else.
- Errors say what happened and what to do, and never blame the person.
- No exclamation marks, no emoji, no congratulation. The application reports; it does not celebrate.
- One word per concept across the whole interface. The vault is "la caja fuerte", everywhere.

## Accessibility

WCAG 2.2 AA is the floor and is not negotiable, because the alternative is an application that stops working the day its author has a bad wrist.

- Everything usable with a mouse is usable with a keyboard, in a logical order, with the focus ring always visible. The ring is defined once, in `base.css`, and is never removed.
- Semantic HTML first. A `div` with an `onClick` is a defect.
- Every control has an accessible name. Every state change outside where the eye already is gets an `aria-live` region.
- Contrast is verified, not estimated. The mark palette is exempt because marks carry no meaning, and that exemption is the reason marks carry no meaning.
- Touch targets are at least 24 by 24 pixels.
- Every screen is walked with `Tab` alone before the work is called done.

## How this is kept true

A design system that lives only in a document is a document. Three things keep this one in the code.

1. **Tokens are the only source of values.** `src/lib/styles/tokens.css` holds them and nothing else defines one. A component needing a value that does not exist adds it there, with a comment saying why, rather than typing a number.
2. **A lint rule fails the build** on a colour literal, an off-scale length or a hand-written duration inside `src/`. Erosion happens one reasonable exception at a time, and a gate is what makes each exception argue for itself.
3. **The review question is fixed**: does every value in this diff come from a token, does the screen work in both themes, does it work with the keyboard alone, and does it work with motion reduced. Four questions, every time.
