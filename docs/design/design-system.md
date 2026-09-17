# Design system

Everything visible in Cairn is built from what is on this page. It is not a style guide to consult when in doubt: it is the source of the values themselves, and a screen that does not come from here is a screen that has to be redone.

Read it before changing anything in `src/`. A colour, a size, a space, a radius or a duration typed by hand into a component is a defect, not a shortcut, and there is a gate that fails the build over it.

## Where the style comes from

The reference is Swiss poster design and the Bauhaus: flat colour, hard edges, a visible grid, geometric marks made of circles and thick rules, one or two saturated colours against paper or near-black, and type that is either very large or very small with nothing much in between.

That language was made for posters, and Cairn is a tool for reading private data every day, so it is translated rather than copied. What comes across:

| Taken | How it appears in an application |
| --- | --- |
| Flat colour, no shadow, no gradient | One border weight separates every surface. There is no elevation scale at all |
| Hard edges | Radii of 2, 4 and 8 pixels. Nothing looks soft, nothing is a pill |
| A saturated accent used with discipline | One vermilion accent, one action per screen. Blue and yellow carry meaning, not decoration |
| Paper and ink | Paper is the base: the light theme is warm cream, not grey. Ink is the variant: near-black, not blue-grey |
| Enormous type against tiny type | A display scale for titles and a tracked-out uppercase label style. The gap between them is the composition |
| Geometric marks | Circles, half circles and thick rules, composed like a poster on the screens that hold no data, and absent from every screen that does |
| The grid, and deliberate asymmetry | Content sits on a real grid, and the title block is allowed to break it |

What does not come across, because a poster is looked at once and an application is looked at a thousand times: full-bleed blocks of saturated colour, colour as the background of anything containing data, decorative shapes anywhere near a number, and compositions that need to be studied.

**Paper is the base and ink is the variant**, and the order matters rather than being a formality. The references are cream paper with saturated figures on top; on near-black the same vermilion, blue and yellow go quiet. The values below are therefore written paper first, and the dark theme is the override. Anybody copying the tokens starts where the references start.

## What the style is

**Expressive minimalism.** The base is quiet to the point of being boring: paper or ink, one-pixel borders, no shadows, no gradients, no decoration that carries no information. Against that base, three things are allowed to be loud: **type**, **one vermilion accent**, and **flat geometry** — and the third one is allowed only on the screens that have no data on them.

That last division is the whole discipline of this system. A screen where somebody reads their own passwords or their own money gets type and one accent and nothing else. A screen where there is nothing to read yet — the lock screen, the first run, an empty list — is where the poster is allowed to happen.

That restraint is not only taste. Cairn has no network, no runtime dependencies and a strict content security policy, so the fashionable ways to add character — a 3D canvas, an animation library, a downloaded icon set, a web font from a CDN — are either impossible or new attack surface in an application that holds a password vault. Type and flat colour cost two bundled files and nothing else.

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
| Gradients, anywhere, including the lock screen | Flat colour is the whole reference. A gradient is the one decoration that always looks like an accident later |

Kept from the current trend list: **bold type**, **dark mode**, **restrained micro-animation**, and **sustainable design** read as what it actually means here — a small bundle, no third-party code, and an interface that works for somebody navigating with a keyboard.

## Colour

Colour carries meaning or it is not used. Surfaces are paper or ink; the accent marks the single most important action on a screen; the status colours never appear without a word beside them, because somebody who cannot tell red from green still has to be able to read the screen.

The Bauhaus triad maps onto the three things this application has to say, which is a happy accident worth keeping: **vermilion** is the accent and means "this is the action", **blue** means information, **yellow** means caution.

### Three families, not two

There are three families of colour token, and each answers a different question.

| Family | Question it answers | May it be text? |
| --- | --- | --- |
| Surfaces, text, accent, status | What is this, and how important is it? | Yes, and every value is contrast-checked |
| `--module-*` | **Which section of the application is this from?** | Yes. A module colour is written as a word as often as it is drawn as a shape |
| `--mark-*` | Nothing. It is a shape on a screen with no data on it | Never |

The middle family is the one that is easy to get wrong. A tab's coloured edge, the square beside a screen title, the top border of a panel card and the left edge of a row in the card picker all say the same thing: this comes from habits, or from passwords, or from finances. That is information, so it cannot be a mark — marks are defined by carrying no meaning, and that definition is what buys them their exemption from contrast. A module colour is held to the same contrast rules as any other text colour.

Each module therefore has three tokens: the colour it is **written** in, the **tint** it is filled with, and the colour its **figure** is drawn in. For three of the four these are the same hue. Finances is the exception and the reason the split exists: full-saturation yellow is unreadable as text on cream, so the word is written in dark ochre and only the shape is yellow.

| Module    | Written in                               | Figure    |
| --------- | ---------------------------------------- | --------- |
| Panel     | vermilion                                | vermilion |
| Habits    | blue                                     | blue      |
| Passwords | ink, inverted to paper on the dark theme | the same  |
| Finances  | dark ochre                               | yellow    |

### Roles

| Token | Role |
| --- | --- |
| `--colour-surface` | The window itself |
| `--colour-surface-raised` | Anything that sits on it: cards, panels, the header |
| `--colour-surface-sunken` | Anything recessed: inputs, the tab strip, the empty part of a chart |
| `--colour-border` | Ordinary separation between surfaces |
| `--colour-border-strong` | Separation that has to be noticed: a focused input, a selected row |
| `--colour-rule` | The thick rule under a section title. Ink, not accent |
| `--colour-text` | What you came to read |
| `--colour-text-muted` | Labels, help, secondary information |
| `--colour-text-faint` | Timestamps, counts, anything read only when looked for |
| `--colour-text-muted-strong` | The same role as muted, on the two screens held to AAA |
| `--colour-accent` | The one important action on the screen, and the focus ring |
| `--colour-accent-strong` | That action under the pointer |
| `--colour-accent-contrast` | Text on top of the accent |
| `--colour-positive` `--colour-warning` `--colour-negative` `--colour-info` | State, always beside a word, always safe as text |
| `--module-*` `--module-*-tint` `--module-*-mark` | Which section something belongs to |
| `--mark-vermilion` `--mark-blue` `--mark-yellow` `--mark-ink` | The poster palette, for geometric marks only. Never text, never behind data |
| `--brand-glyph` | The one place full vermilion is allowed without being an action. See below |

### The brand glyph, and the one exception

Cairn's mark is a cairn: three stacked stones, flat, geometric, drawn in **full vermilion**. It is the application icon, the corner of the title bar and the seal on the lock screen.

This contradicts the rule that vermilion means action, so the exception is named here rather than left to taste, and it is fenced with two prohibitions that hold everywhere:

1. **The glyph never appears next to a primary button.** Two vermilion shapes in one field of view and the rule is gone.
2. **The glyph is never clickable**, and never inside anything clickable. It teaches nothing if it cannot be pressed.

The lock screen is where those two statements meet, because it carries the seal and it also has a button. It is resolved in favour of the rule rather than around it: **on the lock screen the button that opens the vault is not accented.** An accent exists to pick one action out of several, and that screen has exactly one — the button is the only thing on it that can be pressed, so nothing is being distinguished from anything. The seal keeps the vermilion, and the rule that vermilion means action survives intact on the one screen where it would otherwise have had to be excused.

It is `aria-hidden` wherever the product name is written beside it, which is everywhere it appears.

### Values

```css
:root {
  color-scheme: light dark;

  /* Paper. Warm and slightly off, like the references, never pure white. */
  --colour-surface: #f4f1ea;
  --colour-surface-raised: #fbf9f5;
  --colour-surface-sunken: #e9e5db;
  --colour-border: #d9d3c6;
  --colour-border-strong: #b3ab9b;
  --colour-rule: #14130f;

  --colour-text: #14130f;
  --colour-text-muted: #55514a;
  --colour-text-faint: #6b665d;
  --colour-text-muted-strong: #4a4740;

  --colour-accent: #c4380a;
  --colour-accent-strong: #a82e06;
  --colour-accent-contrast: #fbf9f5;

  --colour-positive: #2c7a44;
  --colour-warning: #8a5d10;
  --colour-negative: #b8272c;
  --colour-info: #2450c8;

  --module-panel: #c4380a;
  --module-panel-tint: #fdece4;
  --module-panel-mark: #ff5a1f;
  --module-habits: #1b4fd8;
  --module-habits-tint: #e2e8fa;
  --module-habits-mark: #1b4fd8;
  --module-passwords: #14130f;
  --module-passwords-tint: #e5e2db;
  --module-passwords-mark: #14130f;
  --module-finances: #8a5d10;
  --module-finances-tint: #f8edcb;
  --module-finances-mark: #f2c007;

  --mark-vermilion: #ff5a1f;
  --mark-blue: #1b4fd8;
  --mark-yellow: #f2c007;
  --mark-ink: #14130f;

  --brand-glyph: #c4380a;
}

@media (prefers-color-scheme: dark) {
  :root {
    /* Ink. Near-black and neutral, so the accent is the only warm thing on screen. */
    --colour-surface: #101011;
    --colour-surface-raised: #191a1c;
    --colour-surface-sunken: #0a0a0b;
    --colour-border: #2a2b2e;
    --colour-border-strong: #3e4044;
    --colour-rule: #f2efe9;

    --colour-text: #f2efe9;
    --colour-text-muted: #a5a29b;
    --colour-text-faint: #8a8781;
    --colour-text-muted-strong: #b4b1a9;

    --colour-accent: #ff5a1f;
    --colour-accent-strong: #ff7a45;
    --colour-accent-contrast: #101011;

    --colour-positive: #63c281;
    --colour-warning: #e8b931;
    --colour-negative: #f2696d;
    --colour-info: #5b8cff;

    --module-panel: #ff5a1f;
    --module-panel-tint: #2e1710;
    --module-panel-mark: #ff5a1f;
    --module-habits: #5b8cff;
    --module-habits-tint: #151b2f;
    --module-habits-mark: #5b8cff;
    --module-passwords: #f2efe9;
    --module-passwords-tint: #202022;
    --module-passwords-mark: #f2efe9;
    --module-finances: #e8b931;
    --module-finances-tint: #2a2312;
    --module-finances-mark: #f2c007;

    --mark-ink: #f2efe9;

    --brand-glyph: #ff5a1f;
  }
}
```

The theme is chosen by the system by default and can be overridden from Settings, which sets `data-theme` on the root element. Both overrides repeat the same block, so every value above exists exactly twice in `tokens.css` and nowhere else.

### Measured contrast

Contrast is verified, not estimated, so the numbers are written down. Every pair below was computed against the WCAG 2.x relative luminance formula, on both the base surface and the raised surface, and the worst of the two is what is reported.

| Pair                                     | Paper         | Ink           | Floor |
| ---------------------------------------- | ------------- | ------------- | ----- |
| `--colour-text` on surface               | 16.5:1        | 16.6:1        | 4.5   |
| `--colour-text-muted` on surface         | 7.0:1         | 6.8:1         | 4.5   |
| `--colour-text-faint` on surface         | 5.1:1         | 4.9:1         | 4.5   |
| `--colour-text-muted-strong` on surface  | 8.2:1         | 8.1:1         | 7.0   |
| `--colour-accent` as text                | 4.8:1         | 6.1:1         | 4.5   |
| `--colour-accent-contrast` on the accent | 5.1:1         | 6.1:1         | 4.5   |
| Every status colour as text              | 4.7 to 5.5:1  | 6.0 to 10.3:1 | 4.5   |
| Every module colour as text              | 4.8 to 16.5:1 | 6.0 to 16.6:1 | 4.5   |
| Every module colour on its own tint      | 4.7 to 14.4:1 | 5.4 to 14.2:1 | 4.5   |

Three values changed when they were measured rather than assumed, and the old ones are recorded here so nobody reinstates them: `--colour-accent` on paper was `#dd3f0c`, which reads at 3.9:1 as text and gives near-white on top of it only 4.2:1; `--colour-text-faint` was `#7c776e` on paper and `#6e6c67` on ink, at 3.9:1 and 3.3:1. All three failed AA. The vermilion is still a vermilion; it is two steps darker.

### Rules

- **Contrast is AA everywhere, AAA on the two screens read in a hurry**: the unlock screen and anything showing a password. 4.5:1 for text, 3:1 for interface elements and focus rings, 7:1 for body text on those two screens, which is what `--colour-text-muted-strong` exists for.
- **One accent per screen.** A screen with two accented buttons has not decided what it is for.
- **Status colour is never the only signal.** Red plus the word, not red alone.
- **Mark colours never carry meaning.** If removing the shape would lose information, it was not a mark.
- **Module colours do carry meaning**, are held to AA, and never replace the word they sit beside.
- **No colour literal outside `tokens.css`.** Not in a component, not in a `<style>` block, not "just this once for a border".

## Type

Type is where most of the character lives, so this is the section with the most rules.

### The families

| Token | Used for | Source |
| --- | --- | --- |
| `--font-display` | Screen titles, section names, and numbers that are the point of the screen | **Archivo**, variable, bundled, Latin subset |
| `--font-sans` | Everything else | The system stack: Segoe UI Variable on Windows, San Francisco on iOS |
| `--font-mono` | Amounts, identifiers, key fingerprints, diagnostics | **JetBrains Mono**, variable, bundled, Latin subset |

Archivo is a grotesque in the Swiss line: very large, very tight, very plain, which is exactly what the references do with type. JetBrains Mono is bundled rather than taken from the system stack for one reason that matters in this application: its zero is slashed and its one, its lower-case L and its capital I are unmistakably different from each other, which is what a bank account number needs.

**Both are bundled with the weight axis only.** Archivo also has a width axis, and an earlier draft of this document claimed it "costs nothing extra in a variable font". That was wrong, and it was wrong by a measurable amount: the Latin subset with both axes is 90 KB, and with the weight axis alone it is 35 KB. The width axis was dropped rather than the budget raised. A title is composed by size, weight and letter-spacing here, not by width.

| Asset                                                 | Size    | Budget |
| ----------------------------------------------------- | ------- | ------ |
| `archivo-latin-variable.woff2`, weight 400–700        | 34.9 KB | 80 KB  |
| `jetbrains-mono-latin-variable.woff2`, weight 400–700 | 31.4 KB | 60 KB  |

A bundled face is an asset, not a dependency: it ships as a file in `src/lib/fonts/`, is referenced by a local `@font-face`, and the runtime dependency count stays at zero. Each one ships with its `OFL.txt` beside it. Both must stay subset to Latin, must be variable, and must declare a real fallback in the same stack, so a build that cannot load one still renders a sensible page. That is tested by renaming the file and looking at the result, not by assuming.

Nothing is ever loaded from a font CDN. The content security policy blocks it, and that is correct behaviour rather than an obstacle to work around.

### The scale

A modular scale at a ratio of 1.25 anchored on a 15px body: 15, 19, 24, 30, 38, 47. The three sizes below the body are not on the ratio; they are the sizes a label and a timestamp actually need.

| Token          | Size             | Used for                                      |
| -------------- | ---------------- | --------------------------------------------- |
| `--text-label` | 0.6875rem / 11px | Uppercase tracked labels. See below           |
| `--text-xs`    | 0.75rem / 12px   | Timestamps, counters                          |
| `--text-sm`    | 0.8125rem / 13px | Help text, secondary rows                     |
| `--text-base`  | 0.9375rem / 15px | Body, lists, inputs                           |
| `--text-lg`    | 1.1875rem / 19px | Section subheadings                           |
| `--text-xl`    | 1.5rem / 24px    | Card titles                                   |
| `--text-2xl`   | 1.875rem / 30px  | Dialog titles                                 |
| `--text-3xl`   | 2.375rem / 38px  | Screen titles, and the number on a panel card |
| `--text-4xl`   | 2.9375rem / 47px | The lock screen, and nothing else             |

- **The label style is the signature of this system.** `--text-label`, uppercase, weight 600, `letter-spacing: 0.09em`, in `--colour-text-muted`. It names a section, a field group or a column, and it is the small half of the big-and-small pairing the references are built on.
- Line height: `--leading-tight` (1.1) at `--text-3xl` and above, `--leading-snug` (1.3) between, `--leading-normal` (1.55) for body.
- Letter spacing: `--tracking-display` (-0.03em) on the two display sizes, `--tracking-tight` (-0.01em) at `--text-xl` and `--text-2xl`, none on body, `--tracking-label` (0.09em) on labels.
- Weight: 400 body, 500 labels and buttons, 600 headings, 700 display titles. Nothing lighter than 400 at body size.
- Measure: prose stays within `--measure` (62 characters). Lists and tables take the full width.
- **Never** use size alone where a heading element would do the job properly: the hierarchy has to survive being read aloud.

## Space and shape

- **Space** is a four pixel grid: 4, 8, 12, 16, 24, 32, 48, 64, as `--space-1` to `--space-8`. Nothing in between, ever.
- **Radius** is sharp: `--radius-sm` 2px for inputs and buttons, `--radius-md` 4px for cards and panels, `--radius-lg` 8px for dialogs. Nothing is fully rounded. There are no pills.
- **Borders** are one pixel, `--border-width`, and are the only way surfaces are separated.
- **The rule** is the thick line the references use to anchor a title block: `--rule-width` 3px in `--colour-rule`, under a screen title and nowhere else. It is the one piece of pure typographic furniture in the system.
- **Elevation does not exist.** No shadow scale. A panel above the page is a raised surface with a border. A dialog additionally dims what is behind it.
- **Density** is single and comfortable. A compact mode is a decision for the day there are long lists to justify it.
- **Content is capped** at `--content-max` (1100px) and centred in what is left.

### The two widths that are not tokens

Media query preludes cannot read a custom property, so the two breakpoints are written as numbers in the query itself, and they are the only numbers in `src/` allowed to be.

| Width | What changes |
| --- | --- |
| 880px | The window minimum. Below it the geometric compositions are removed and the tab cap drops from six to four |
| 420px | The phone shape. Below it no mark is drawn at all |

## Geometric marks

Circles, half circles, thick rules and the occasional square, flat and from the mark palette. They are the part of the references that gives the application a face, and they are also the part that can ruin it, so the rule is not "use them tastefully": it is a list of places, a budget, and four prohibitions that hold everywhere.

### Where they are allowed

Six places, and no seventh without amending this table.

| Place | Budget | What it looks like |
| --- | --- | --- |
| The lock screen | Up to four shapes | The full composition. A large vermilion circle breaking the top right corner, a blue half circle sitting on its lower edge, an ink rule crossing the width, one small yellow dot low on the opposite side. Fixed: the same arrangement every time, because this screen is seen twice a day and a composition that moves is a composition that irritates |
| The panel header | Up to three shapes | The panel is the first thing seen after unlocking and was the only screen without a face. A reduced version of the lock screen composition, in its own region to the right of the title block, with nothing on top of it |
| Creating a vault, and the damaged-header screen | Up to two shapes | A further reduced version. They are the other two screens with no data on them |
| An empty state | Up to three shapes | Its own small composition, no more than 160px across, above the sentence |
| A module section header | Exactly one | A small filled square before the label, in that module's figure colour |
| The corner of the title bar | Exactly one | The brand glyph, beside the product name |

### Where they are never allowed

Anywhere with data on it. A list, a table, a card showing a value, a form, a dialog, a chart, a row, a total. No exceptions, and not "faintly in the background either": a number is read, and anything behind it competes for the same attention.

### The four prohibitions

1. **Nothing is ever placed on top of a mark, and a mark is never placed on top of anything.** Geometry lives in its own region of the layout. Where the window is too narrow for that region, the mark is removed, not shrunk into the content: below 880px the composition drops to one shape, below 420px to none.
2. **A mark is never animated**, never on load, never on hover, never on the way in.
3. **A mark never responds to the pointer.** `pointer-events: none`, always.
4. **A mark never means anything.** It is `aria-hidden`, it is hidden in forced-colors mode, and a screen that reads worse without it was never decorated — it was under-labelled, and the fix is a word.

Flat fill only, no stroke unless the shape is itself a rule, no gradient, no transparency, no overlap that produces a third colour. If somebody has to look twice to work out whether a shape means something, the shape is wrong.

## Motion

Motion explains a change of state; it never decorates one. Only three things move: something appearing, something disappearing, and the focus ring.

- `--duration-fast` 90ms for anything under the pointer: hover, press, focus.
- `--duration-normal` 160ms for something appearing or disappearing.
- Nothing is slower than 250ms. This application is opened fifty times a day and anything above that reads as lag.
- **Changing tab does not animate**, and neither does anything in a list. A view that slides is a view somebody is waiting for.
- Only `transform` and `opacity` are animated. Animating width, height or colour across a list is how a fast interface becomes a slow one.
- `prefers-reduced-motion: reduce` sets every duration to zero, once, globally, in `tokens.css`, so nobody has to remember it per component.

## Icons

Icons are hand-written inline SVG in Svelte components under `src/lib/icons/`. There is no icon package, because an icon package is a runtime dependency and the dependency count is a gate.

- 24px box, 1.5px stroke, round caps, no fill, geometric construction to match the type.
- `currentColor` always, so an icon inherits the colour of what it belongs to.
- Every icon that is not decorative carries an accessible name; every decorative one is hidden from assistive technology.
- An icon never stands alone on a control whose meaning is not obvious from context. When in doubt, a word.

There are **eighteen**, and the list does not grow without a line in this table saying what the new one is for.

| Icon | What it marks |
| --- | --- |
| Panel, Habits, Passwords, Finances | The four sections, in the menu, the palette and the tab strip |
| Settings, Diagnostics | The two administrative screens |
| Lock closed, Lock open | The state of the vault in the header |
| Plus | Open something, add a card |
| Close | Close a tab, remove a card, dismiss a layer |
| Pin | A tab that has been made permanent |
| Clock | Session history, and reopening the last closed tab |
| Search | The search field in the palette |
| Chevron | A menu that opens downwards |
| Undo | Undo a deletion while the few seconds last |
| Window minimise, Window maximise, Window close | The three controls of the undecorated window |

## Components

Rules, not a catalogue. The catalogue is the code.

**Buttons.** At most one accented button per screen: solid vermilion, `--colour-accent-contrast` text, 2px radius. Everything else is a bordered button on a raised surface, or plain text underlined on hover. A disabled button explains why, through its `title` and, where it matters, in visible text: a control that is dead for an unexplained reason is a bug report waiting to be filed.

**Inputs.** Label above in the label style, always a real `<label>`, never a placeholder standing in for one. Help below in `--colour-text-muted`, errors below that in `--colour-negative`. Sunken surface, one pixel border, `--colour-border-strong` plus the focus ring when focused.

**Cards and panels.** Raised surface, one pixel border, a 6px top edge in the owning module's colour, `--radius-md` on the bottom corners only, `--space-5` of padding. No shadow.

**Lists.** Rows separated by a one pixel border, never by alternating background. Hover raises the surface rather than tinting it. A selected row carries `--colour-border-strong` and a 3px accent edge on the left.

**Tabs.** The active tab carries a 3px top edge and a tinted background, both in its module's colour. A temporary tab is in italics with a dotted underline. A title is truncated by CSS and trimmed at the source: a tab is a strip of window, and the thing written in it will one day be a name the person chose. A tab is made permanent with a double click or with `Shift` `Enter`, and moved by dragging or with `Shift` and an arrow — every gesture in the strip has a keyboard equivalent, which is the one promise the undecorated window already costs us once and is not allowed to cost us twice.

**Layers.** The **Abrir** menu, the card picker and the command palette are all layers: they open in place under or over what opened them, they close on `Escape` with the focus going back where it came from, and they close when the focus leaves them. None of them is a dialog, because none of them asks a question that carrying on cannot undo — putting a card on the panel is undone by taking it off. The palette is the exception that is drawn over the whole window, and the only one that keeps the focus inside, because it is the whole of what somebody is doing while it is open. There is no dimming layer behind any of them: a tint over the window would be a colour that is not a token and a second thing competing for the eye, and a raised surface with a border is enough.

**Cards on the panel.** A card says what it is and never what it holds. Not a balance, not a user name, not whether a habit was done. The panel is the first thing on screen after unlocking and the thing visible to anybody walking past a window somebody stepped away from, and the same rule governs a search result for the same reason.

**Empty states.** Every list has one, and it is the real thing the person sees on their first day rather than a placeholder: a geometric mark, one sentence saying what will be here, and the action that puts something there. Never the word "empty" on its own.

**"In development".** Anything drawn before it works says so, in a small uppercase badge in its own module's colour, in the place the value will go. A button that is not wired up is still shown and still reacts: pressing it says the part is in development. A control that does nothing looks like a broken application; one that explains looks like an unfinished one, which is what it is.

**Warnings that matter.** The ones about losing data are the exception to every rule about restraint on this page: they go before the fields rather than after, they carry `--colour-warning` on the border and the title, and they say the whole thing in plain Spanish. A warning under a form is a warning read after the decision.

**Dialogs.** Only for a choice that carrying on cannot undo. Everything else happens in place, and a deletion is undone from a strip that lasts a few seconds rather than confirmed in advance. A dialog traps focus, closes on `Escape`, returns focus where it came from, and never appears unasked.

**Loading.** A discreet indicator that appears only after 200ms. No skeletons: against a local database almost everything is instant, and a skeleton that flashes for 40ms reads as a fault.

**Errors.** Inside the screen where they happened, next to what caused them. Never a floating notice that takes itself away, which is an error nobody finished reading.

## Layout

The application is a header, a tab strip and content, in that order down the window. There is no sidebar; navigation is by tabs, which is what the header and the strip are for.

- **The header** is the top row and, because the window has no system decoration, it is also the drag region. It holds the brand glyph and name, the **Abrir** menu, the state of the vault with its countdown, the reminder of the palette shortcut, the button that closes the vault, and the three window controls.
- **The tab strip** sits directly under it and is drawn as part of the same piece: one border between them and none above. The panel is always the first tab and cannot be closed.
- **Content** is capped at `--content-max` and centred in what is left. A screen title sits in a block of its own above it, with the rule under it, and the geometric region — where a screen is allowed one — sits to its right.
- Below 880px the geometry goes and the tab cap drops to four. Below 420px no mark is drawn. The header keeps the menu, the vault state and the palette reminder at every width, and below 880px the button that closes the vault gives up its words for its glyph: it is the widest thing in the row and a padlock survives losing them, while `Ctrl` `K` is two keys with nothing to shorten it to and is what tells somebody the palette exists at all.
- **The lock screen has no shell.** It is the boundary between being in and being out, it fills the window, and it is the one screen built like a poster: the form on one side, the full composition on the other.
- **Every screen that fills the window still gets the title bar**, drawn with no background and holding only the three window controls. The window has no system decoration, so a screen without it would be a screen the window could not be moved or closed from — on the lock screen, a trap.
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
- Every control has an accessible name.
- Contrast is verified, not estimated, and the numbers are in the table above. The mark palette is exempt because marks carry no meaning, and that exemption is the reason marks carry no meaning.
- Touch targets are at least 24 by 24 pixels.
- Every screen is walked with `Tab` alone before the work is called done, and `axe` runs over every preview screen in both themes as a blocking gate.

**What this deliberately does not do.** There is no `aria-live` region announcing a form error or the lock countdown, because the person this is built for does not use a screen reader and the honest choice was to do the AA floor well rather than half of AAA badly. It is written here rather than left to be discovered: adding announcements later means walking every screen again.

**And one thing it costs.** The window has no system decoration, so moving it is a pointer gesture on the header with no keyboard equivalent. The window can still be resized from its edges, which the platform handles, and closed from the keyboard through the control at the end of the bar — but somebody who cannot use a pointer cannot move it. That is a real regression against a system title bar, and it is recorded rather than hidden in [decision 0007](../architecture/decisions/0007-undecorated-window.md).

## How this is kept true

A design system that lives only in a document is a document. Three things keep this one in the code.

1. **Tokens are the only source of values.** `src/lib/styles/tokens.css` holds them and nothing else defines one. A component needing a value that does not exist adds it there, with a comment saying why, rather than typing a number.
2. **A gate fails the build** on a colour literal, a `px` or `rem` length, or a duration written by hand anywhere in `src/` outside `tokens.css`. It is `node scripts/check-tokens.mjs`, it has no dependencies, and its list of exemptions starts empty. An exemption is a comment on the line above saying why, so each one argues for itself in the diff.
3. **The review question is fixed**: does every value in this diff come from a token, does the screen work in both themes, does it work with the keyboard alone, and does it work with motion reduced. Four questions, every time.
