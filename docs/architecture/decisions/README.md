# Decision records

A record is written here when a decision is expensive to reverse: the data format, the cryptographic model, the boundaries between modules, the synchronisation strategy, a structural dependency.

Each one says what the situation was, what was chosen, what was rejected and why, and what it costs. The costs are written with the same honesty as the benefits. A record that only lists advantages is not a record of a decision, it is an advertisement for one, and it is no use at all to the person who later has to work out whether the reasoning still holds.

| # | Decision | Status |
| --- | --- | --- |
| [0001](0001-workspace-and-crate-boundaries.md) | A workspace of five crates, all created before they are needed | accepted |
| [0002](0002-svelte-and-no-runtime-dependencies.md) | Svelte, hand written CSS, and no runtime dependencies | accepted |
| [0003](0003-wrapped-data-key.md) | A wrapped data key, with every other key derived from it | accepted |
| [0004](0004-no-recovery-and-one-unlock-error.md) | No recovery path, and one indistinguishable unlock failure | accepted |
| [0005](0005-argon2-parameters-and-platform-storage.md) | Argon2id parameters in the header, and the shape of platform storage | accepted |
| [0006](0006-tabbed-navigation-and-bundled-type.md) | Tabbed navigation, and typefaces bundled as assets | accepted |

A record is never edited to say something different once it is accepted. If a decision is replaced, the new one gets its own number and the old one is marked as superseded, so that the reasoning behind a change is still readable afterwards.
