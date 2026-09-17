//! How deep a tree of folders is allowed to get, and why the check is here.
//!
//! The name of a folder is encrypted, so SQL cannot walk the tree: a recursive query would have
//! to compare values it cannot read. What it can do is follow the parent column, which is in the
//! clear, and that is exactly what this does — over identifiers, with no names in it at all.
//!
//! A pure function over a list of parents, so the rule can be checked against ten thousand
//! generated shapes rather than against the three somebody thought of. The two failures it has
//! to survive are a chain longer than the limit and a cycle, and a cycle is not hypothetical:
//! two devices can each reparent a folder under the other's, and a merge that accepts both
//! produces a loop that no amount of care at the point of insertion would have prevented.

/// How many levels of folders the design allows.
///
/// Eight. Deep enough that nobody organising passwords runs into it, shallow enough that walking
/// a path is bounded work, and the bound is what makes it safe to walk one at all.
pub const MAX_DEPTH: usize = 8;

/// What is wrong with a placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TreeError {
    /// The folder would sit deeper than [`MAX_DEPTH`] allows.
    #[error("the folder would be at depth {depth}, and at most {MAX_DEPTH} is allowed")]
    TooDeep {
        /// How deep it would be, counting the root as one.
        depth: usize,
    },

    /// Following the parents came back to where it started.
    ///
    /// Not a hypothetical. Two devices can each move a folder under the other's while they are
    /// apart, and a merge that takes both writes produces a loop. Reported rather than followed,
    /// because following it is an application that stops responding.
    #[error("the folders form a loop")]
    Cycle,
}

/// Looks up the parent of a folder, or `None` for one at the root.
///
/// A trait over a closure rather than a map, so the caller can answer from a query, from a cache
/// or from a literal without this module knowing which.
pub trait Parents {
    /// The parent of a folder, by whatever identifier the caller uses.
    fn parent_of(&self, folder: u128) -> Option<u128>;
}

impl<F> Parents for F
where
    F: Fn(u128) -> Option<u128>,
{
    fn parent_of(&self, folder: u128) -> Option<u128> {
        self(folder)
    }
}

/// How deep a folder sits, counting itself as one.
///
/// # Errors
///
/// Returns [`TreeError::Cycle`] if following the parents comes back to a folder already seen, and
/// [`TreeError::TooDeep`] if the chain is longer than [`MAX_DEPTH`].
pub fn depth_of(folder: u128, parents: &impl Parents) -> Result<usize, TreeError> {
    let mut seen = [0_u128; MAX_DEPTH];
    let mut depth = 0_usize;
    let mut current = folder;

    loop {
        // Checked against everything already walked rather than against the starting point
        // alone. A loop that does not include the folder being asked about is still a loop, and
        // it is the shape a merge produces most often.
        if seen.iter().take(depth).any(|earlier| *earlier == current) {
            return Err(TreeError::Cycle);
        }

        if depth == MAX_DEPTH {
            return Err(TreeError::TooDeep { depth: depth + 1 });
        }

        // Bounded by the check above, so the slot is always inside the array.
        if let Some(slot) = seen.get_mut(depth) {
            *slot = current;
        }
        depth += 1;

        match parents.parent_of(current) {
            Some(parent) => current = parent,
            None => return Ok(depth),
        }
    }
}

/// Whether a folder may be placed under a parent.
///
/// The check to run before writing a move. Takes the parent rather than the folder, because the
/// question is about where it is going, and asking after the write is asking too late.
///
/// # Errors
///
/// Returns [`TreeError::TooDeep`] if the placement would put the folder past [`MAX_DEPTH`], and
/// [`TreeError::Cycle`] if the parent's own chain already loops.
pub fn may_place_under(parent: Option<u128>, parents: &impl Parents) -> Result<(), TreeError> {
    let Some(parent) = parent else {
        return Ok(());
    };

    let depth = depth_of(parent, parents)?;
    if depth >= MAX_DEPTH {
        return Err(TreeError::TooDeep { depth: depth + 1 });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{MAX_DEPTH, TreeError, depth_of, may_place_under};

    /// A chain of folders numbered one upwards, each under the one before it.
    fn a_chain(length: u128) -> impl Fn(u128) -> Option<u128> {
        move |folder| {
            if folder <= 1 || folder > length {
                None
            } else {
                Some(folder - 1)
            }
        }
    }

    #[test]
    fn a_folder_at_the_root_is_at_depth_one() {
        let parents = |_folder: u128| None;

        assert_eq!(depth_of(7, &parents), Ok(1));
        assert_eq!(may_place_under(None, &parents), Ok(()));
    }

    #[test]
    fn a_chain_as_long_as_the_limit_is_accepted_and_one_longer_is_not() {
        let allowed = a_chain(MAX_DEPTH as u128);
        assert_eq!(depth_of(MAX_DEPTH as u128, &allowed), Ok(MAX_DEPTH));

        // Placing anything under the deepest folder allowed is what the limit refuses, and it is
        // the question the application actually asks: never "how deep is this", always "may this
        // go there".
        assert_eq!(
            may_place_under(Some(MAX_DEPTH as u128), &allowed),
            Err(TreeError::TooDeep {
                depth: MAX_DEPTH + 1
            })
        );

        let too_long = a_chain(MAX_DEPTH as u128 + 1);
        assert_eq!(
            depth_of(MAX_DEPTH as u128 + 1, &too_long),
            Err(TreeError::TooDeep {
                depth: MAX_DEPTH + 1
            })
        );
    }

    #[test]
    fn a_folder_that_is_its_own_parent_is_a_loop_rather_than_an_infinite_walk() {
        let parents = |folder: u128| Some(folder);

        assert_eq!(depth_of(1, &parents), Err(TreeError::Cycle));
    }

    #[test]
    fn two_folders_under_each_other_are_a_loop() {
        // The shape a merge produces: each device moved one folder under the other while they
        // were apart, and both writes are legitimate on their own.
        let parents = |folder: u128| Some(if folder == 1 { 2 } else { 1 });

        assert_eq!(depth_of(1, &parents), Err(TreeError::Cycle));
        assert_eq!(may_place_under(Some(1), &parents), Err(TreeError::Cycle));
    }

    #[test]
    fn a_loop_that_does_not_include_the_folder_asked_about_is_still_found() {
        // Three under two, two under three, and one under three. Walking from one never returns
        // to one, so a check that only watched the starting point would walk for ever.
        let parents = |folder: u128| match folder {
            1 | 2 => Some(3),
            3 => Some(2),
            _ => None,
        };

        assert_eq!(depth_of(1, &parents), Err(TreeError::Cycle));
    }

    proptest! {
        /// Any chain of parents terminates, with an answer or with a refusal, and never walks
        /// for ever. The property that matters is not which answer: it is that there is one.
        #[test]
        fn every_shape_of_parents_answers_rather_than_looping(
            parents in proptest::collection::vec(proptest::option::of(0_u128..12), 1..12),
        ) {
            let lookup = move |folder: u128| {
                usize::try_from(folder)
                    .ok()
                    .and_then(|index| parents.get(index))
                    .copied()
                    .flatten()
            };

            for folder in 0..12 {
                let answer = depth_of(folder, &lookup);
                prop_assert!(
                    answer.is_err() || answer.is_ok_and(|depth| (1..=MAX_DEPTH).contains(&depth)),
                    "a depth outside the allowed range was accepted"
                );
            }
        }

        /// A placement that is accepted leaves the child inside the limit. This is the whole
        /// contract: the check is run before the write precisely so that the tree afterwards
        /// satisfies the rule, and an accepted placement whose child was too deep would make
        /// the check decorative.
        #[test]
        fn an_accepted_placement_leaves_the_child_inside_the_limit(length in 0_u128..12) {
            let parents = a_chain(length);

            let parent = if length == 0 { None } else { Some(length) };
            if may_place_under(parent, &parents).is_err() {
                return Ok(());
            }

            let depth = parent.map_or(0, |folder| depth_of(folder, &parents).unwrap_or(MAX_DEPTH));
            prop_assert!(depth < MAX_DEPTH);
        }
    }
}
