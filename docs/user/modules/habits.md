# Habits

A habit in Cairn is one thing you want to do, or stop doing, tracked one calendar day at a time. There is a list of what today asks for, a page per habit with its whole history, and a form to change any of it. Nothing leaves your machine, and nothing about a habit is shared with anything.

This page explains what the application decides on your behalf, because a habit tracker is mostly arithmetic you did not ask to see, and the arithmetic is only trustworthy if you can check it.

## The three kinds

Every habit is one of three shapes. You choose the shape by filling in the form, not by picking from a list of types.

**Done or not done.** Meditate. Read. Take the pills. There is nothing to count: the day was either done or it was not. Leave the unit empty and this is what you get.

**A quantity.** Two litres of water, thirty pages, five kilometres. Name a unit and a target, and the day is met when you reach the target. The amount is always a whole number in the smallest unit you care about: two and a half kilometres is 2500 metres, not 2.5 kilometres. Fractions in a column of numbers add up differently depending on the order they are added in, and a total that changes when nothing changed is not worth showing.

**Something you want to avoid.** Not smoking. No sugar. Here the good day is the day you do not mark, and marking is recording a slip. The button says so: it offers to note a slip rather than to mark the day done.

Each of the three can be judged **every day** or **by the week**. A weekly habit has a target in days — three runs a week — and the week as a whole is what passes or fails, not each day inside it.

## What "done" means, exactly

A day gets one of five verdicts, and everything else on the screen — the streak, the month, the colour of a square — is one of those five read a different way.

| The day says               | When                                                     |
| -------------------------- | -------------------------------------------------------- |
| **Done**                   | The habit asked for that day and you met it              |
| **Missed**                 | The habit asked for that day and you did not             |
| **Beyond what was asked**  | You did it on a day the habit never asked about          |
| **Not asked for that day** | A weekday outside your schedule, with nothing done on it |
| **Nothing to say**         | Before the habit started, or after today                 |

For a habit you are building, meeting the day means there is a mark and its amount reaches the target. No mark at all means nothing happened.

For a habit you are avoiding, it is the other way round and deliberately not a mirror image: the day is met by the _absence_ of a mark. Not smoking today is not something you do; it is something that did not happen, and the application counts it in your favour without you touching anything.

## The streak, and when it breaks

The streak is the number people look at, so it is worth saying precisely what it counts.

**Only a day the habit asked for can break it.** If you run on Mondays, Wednesdays and Fridays, a Tuesday does nothing at all to the streak. It is not a missed day; it is not a day. A tracker that counted it would break your streak four times a week.

**Today never breaks it.** A day you are still living is not a day you failed. Until midnight — or until your day ends, see below — an unmarked habit shows as _at risk_, which means you still have time. The streak stays at its number. It breaks the following day, once the day it belonged to is over.

**A day you did anyway counts.** Training on a Tuesday when you only train on Mondays adds to the streak instead of being ignored. Refusing to count it would punish the extra session.

**A weekly habit counts weeks, not days.** The week in progress is taken out of the count and shown on its own — _one of three this week, which is still open_ — because one of three on a Tuesday is not being behind, it is being at the beginning. The week only ends the streak once it is over and short.

Weeks run Monday to Sunday, the ISO week, everywhere in the application.

## The month percentage

The number under a habit is the days it was met over the days it asked for **and that are already over**. It is written as both numbers and the percentage, never the percentage alone, because ninety per cent of an unknown number of days is a figure you cannot act on.

Today is not in the denominator unless you have already met it. That is the same rule the streak uses, for the same reason: a day in progress neither succeeded nor failed, so it stays out of the division.

A month can read above a hundred per cent. That is the honest reading of having done more days than were asked of you.

## The year at a glance

Each habit draws its year as fifty-three columns of seven days, Monday at the top. The five verdicts differ by shape as well as by colour, so the picture is readable without depending on colour at all.

| The square                       | What it means                              |
| -------------------------------- | ------------------------------------------ |
| Filled                           | Done                                       |
| Filled with a hole in the middle | Beyond what was asked                      |
| Outlined, empty inside           | Missed                                     |
| A dot on its own                 | The habit did not ask for that day         |
| Dashed outline                   | Before the habit existed, or still to come |

The arrows step a year at a time. The one going back stops at the first year you have anything marked in, and the one going forward stops at the year in progress; both stay on screen when they are unavailable and say why, because a control that disappears is a control you go looking for.

## Editing a habit that already has history

This is where a habit tracker either keeps your record or quietly rewrites it. Cairn keeps it.

**Raising or lowering a target does not repaint the past.** A day is judged against the target that was in force the day you marked it, and it keeps that verdict forever. Raise your water goal from two litres to three and every two-litre day stays green. Nothing about a year you actually lived changes because of a decision you took this morning.

**Changing days does not delete marks.** If you drop Saturday from a habit's schedule, the Saturdays you already marked stay on the calendar — drawn as days the habit no longer asks for. They stop counting; they do not disappear.

**Changing what a habit means warns you first.** Turning a habit you were building into one you are avoiding changes the meaning of every day behind it, and therefore the streak. Before saving, the application shows what the streak is now and what it would become, and nothing is written until you accept. Cancelling, or pressing `Esc`, changes nothing at all.

## Marking a day that is not today

You can fill in the last **thirty days**, no further. Far enough to catch up a week you forgot, near enough that the history is a history rather than something anybody can rewrite at will.

A day that has not happened yet cannot be marked at all.

## Archiving and deleting

**Archiving** takes a habit off the list of what today asks for and keeps everything else: the marks, the streak, the whole calendar. Bringing it back restores the streak at the number it had, not at zero. This is what to do with a habit you have stopped.

**Deleting** removes the habit and every day you marked in it, and cannot be undone. The confirmation says how many days that is, and offers to archive it instead.

## When your day starts

If you are awake at one in the morning, the habit you are marking probably belongs to yesterday. Cairn can be told that your day starts at, say, four, and then everything on this page — which day is today, what the streak counts, what the month divides by — moves with it.

The preference exists and the core reads it, but **there is no screen that sets it yet**. Until there is, days start at midnight in the time zone your machine reports.

Changing your machine's time zone never rewrites a day you have already marked. A marked day is a date on a calendar, not a moment in time, so it means the same thing on a laptop in Madrid and on the same file opened in Tokyo. What does change, correctly, is which day is _today_.

## What is not here yet

Written plainly, because a feature that is planned and a feature that exists should never be hard to tell apart.

- **Grouping habits into areas** — the file has somewhere to put it, no screen uses it.
- **Pauses** — the stretch of days where a missed day should not break a streak, for an illness or a holiday. Same: room in the file, nothing that writes it.
- **Setting when your day starts** — read by the core, not yet settable.
- **Marking a past day from the calendar** — the year is drawn but its squares cannot be pressed. Only today can be marked from the screen.
- **A note on a single day** — the file has a column for it; nothing reads or writes it. Only the habit itself has notes.
