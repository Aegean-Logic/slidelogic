//! The order of a list of things with ids (slides, masters, the shapes on a slide): the edits that
//! turn one order into another, applying them, and moving them past someone else's.
//!
//! Places are named by neighbour, not by index: "after slide 7". Someone else's edit made at the
//! same time can't shift that, only remove or move the neighbour, and [`rebase`] then names the
//! neighbour's own old neighbour instead.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use serde::{Deserialize, Serialize};

/// One change to the order of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum SeqEdit<I> {
    /// `id`, new, goes after `after` (`None`: first).
    Insert { id: I, after: Option<I> },
    /// `id` moves to after `after`; it was after `from`.
    Move { id: I, after: Option<I>, from: Option<I> },
    /// `id` goes; it was after `after`.
    Remove { id: I, after: Option<I> },
}

impl<I: Copy> SeqEdit<I> {
    pub fn id(&self) -> I {
        match *self {
            SeqEdit::Insert { id, .. } | SeqEdit::Move { id, .. } | SeqEdit::Remove { id, .. } => id,
        }
    }
}

/// The edits that turn the order `before` into `after`: removals, then every new or moved id in
/// its new order, each placed after its new neighbour. `None` when either list repeats an id.
pub fn diff<I: Copy + Eq + Hash>(before: &[I], after: &[I]) -> Option<Vec<SeqEdit<I>>> {
    let at: HashMap<I, usize> = before.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let now: HashSet<I> = after.iter().copied().collect();
    if at.len() != before.len() || now.len() != after.len() {
        return None;
    }
    let prev = |list: &[I], i: usize| i.checked_sub(1).and_then(|j| list.get(j)).copied();
    let mut out = Vec::new();
    for (i, id) in before.iter().enumerate() {
        if !now.contains(id) {
            out.push(SeqEdit::Remove { id: *id, after: prev(before, i) });
        }
    }
    // The ids that stay put: the longest run of kept ids whose order didn't change.
    let kept: Vec<(I, usize)> = after.iter().filter_map(|id| at.get(id).map(|&i| (*id, i))).collect();
    let stay: HashSet<I> =
        longest_increasing(&kept.iter().map(|(_, i)| *i).collect::<Vec<_>>()).into_iter().filter_map(|k| kept.get(k).map(|(id, _)| *id)).collect();
    let mut last = None;
    for id in after {
        match at.get(id) {
            None => out.push(SeqEdit::Insert { id: *id, after: last }),
            Some(&i) if !stay.contains(id) => out.push(SeqEdit::Move { id: *id, after: last, from: prev(before, i) }),
            Some(_) => {}
        }
        last = Some(*id);
    }
    Some(out)
}

/// Indexes into `v` of a longest strictly increasing subsequence (patience sorting).
fn longest_increasing(v: &[usize]) -> Vec<usize> {
    // tails[k]: index of the smallest last value of an increasing run of length k + 1.
    let mut tails: Vec<usize> = Vec::new();
    let mut back: Vec<Option<usize>> = vec![None; v.len()];
    for (i, x) in v.iter().enumerate() {
        let k = tails.partition_point(|&t| v.get(t).is_some_and(|y| y < x));
        if let Some(b) = back.get_mut(i) {
            *b = k.checked_sub(1).and_then(|j| tails.get(j)).copied();
        }
        match tails.get_mut(k) {
            Some(t) => *t = i,
            None => tails.push(i),
        }
    }
    let mut out = Vec::with_capacity(tails.len());
    let mut cur = tails.last().copied();
    while let Some(i) = cur {
        out.push(i);
        cur = back.get(i).copied().flatten();
    }
    out.reverse();
    out
}

/// Applies `edits` to `items`. `new` gives the item an insert names (`None` skips the insert).
/// Edits that no longer fit (an id already there, or gone) are skipped; a neighbour that's gone
/// puts the item last. Every copy skips the same ones, so they stay the same.
pub fn apply<I: Copy + Eq, T>(items: &mut Vec<T>, edits: &[SeqEdit<I>], id_of: impl Fn(&T) -> I, mut new: impl FnMut(I) -> Option<T>) {
    let pos = |items: &[T], id: I| items.iter().position(|t| id_of(t) == id);
    let place = |items: &[T], after: Option<I>| match after {
        None => 0,
        Some(a) => pos(items, a).map_or(items.len(), |i| i + 1),
    };
    for e in edits {
        match *e {
            SeqEdit::Remove { id, .. } => {
                if let Some(i) = pos(items, id) {
                    items.remove(i);
                }
            }
            SeqEdit::Insert { id, after } => {
                if pos(items, id).is_none()
                    && let Some(t) = new(id)
                {
                    let at = place(items, after);
                    items.insert(at, t);
                }
            }
            SeqEdit::Move { id, after, .. } => {
                if after != Some(id)
                    && let Some(i) = pos(items, id)
                {
                    let t = items.remove(i);
                    let at = place(items, after);
                    items.insert(at, t);
                }
            }
        }
    }
}

/// `a` rebased to apply after `b` (both made against the same list). `a_first`: `a` comes
/// earlier in the log, so where both move the same thing `b`'s move stands, and where both put
/// things at the same place `b`'s go first (as if `a` had gone in before `b`).
pub fn rebase<I: Copy + Eq + Hash>(a: &[SeqEdit<I>], b: &[SeqEdit<I>], a_first: bool) -> Vec<SeqEdit<I>> {
    let mut removed: HashMap<I, Option<I>> = HashMap::new();
    let mut moved: HashMap<I, Option<I>> = HashMap::new();
    let mut inserted: HashSet<I> = HashSet::new();
    for e in b {
        match *e {
            SeqEdit::Remove { id, after } => {
                removed.insert(id, after);
            }
            SeqEdit::Move { id, from, .. } => {
                moved.entry(id).or_insert(from);
            }
            SeqEdit::Insert { id, .. } => {
                inserted.insert(id);
            }
        }
    }
    // A neighbour `b` removed or moved: its old neighbour, and so on.
    let chase = |mut at: Option<I>| {
        for _ in 0..=b.len() {
            match at {
                Some(x) if removed.contains_key(&x) => at = removed.get(&x).copied().flatten(),
                Some(x) if moved.contains_key(&x) => at = moved.get(&x).copied().flatten(),
                _ => break,
            }
        }
        at
    };
    // `a` first: what `b` put right after the same neighbour goes before `a`'s.
    let past_b = |id: I, mut at: Option<I>| {
        if !a_first {
            return at;
        }
        for _ in 0..b.len() {
            let next = b.iter().find_map(|e| match *e {
                SeqEdit::Insert { id: z, after } | SeqEdit::Move { id: z, after, .. } if after == at && z != id => Some(z),
                _ => None,
            });
            match next {
                Some(z) => at = Some(z),
                None => break,
            }
        }
        at
    };
    let mut out = Vec::with_capacity(a.len());
    for e in a {
        match *e {
            SeqEdit::Remove { id, after } => {
                if !removed.contains_key(&id) {
                    out.push(SeqEdit::Remove { id, after: chase(after) });
                }
            }
            SeqEdit::Insert { id, after } => {
                if !inserted.contains(&id) && !removed.contains_key(&id) {
                    out.push(SeqEdit::Insert { id, after: past_b(id, chase(after)) });
                }
            }
            SeqEdit::Move { id, after, from } => {
                if !(removed.contains_key(&id) || (a_first && moved.contains_key(&id))) {
                    out.push(SeqEdit::Move { id, after: past_b(id, chase(after)), from: chase(from) });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(list: &[u32], edits: &[SeqEdit<u32>]) -> Vec<u32> {
        let mut v = list.to_vec();
        apply(&mut v, edits, |x| *x, Some);
        v
    }

    #[test]
    fn diff_then_apply_gives_the_new_order() {
        let cases: &[(&[u32], &[u32])] =
            &[(&[1, 2, 3, 4], &[4, 3, 2, 1]), (&[1, 2, 3], &[3, 9, 1]), (&[], &[5, 6]), (&[1, 2, 3], &[]), (&[1, 2, 3, 4, 5], &[2, 3, 1, 5, 4, 7])];
        for (before, after) in cases {
            let edits = diff(before, after).unwrap();
            assert_eq!(run(before, &edits), after.to_vec(), "{before:?} -> {after:?}: {edits:?}");
        }
        // Only what moved is moved.
        let edits = diff(&[1, 2, 3, 4, 5], &[1, 3, 4, 5, 2]).unwrap();
        assert_eq!(edits, vec![SeqEdit::Move { id: 2, after: Some(5), from: Some(1) }]);
        assert!(diff(&[1, 1], &[1]).is_none());
    }

    #[test]
    fn longest_increasing_run() {
        assert_eq!(longest_increasing(&[3, 1, 2, 5, 4]).len(), 3);
        assert_eq!(longest_increasing(&[]), Vec::<usize>::new());
        assert_eq!(longest_increasing(&[0, 1, 2]), vec![0, 1, 2]);
    }

    /// Every edit one person can make to a short list.
    fn single_edits(list: &[u32], new: u32) -> Vec<Vec<u32>> {
        let mut out = Vec::new();
        for i in 0..list.len() {
            let mut v = list.to_vec();
            v.remove(i);
            out.push(v.clone());
            for j in 0..=v.len() {
                let mut m = v.clone();
                m.insert(j, list[i]);
                if m != list {
                    out.push(m);
                }
            }
        }
        for j in 0..=list.len() {
            let mut v = list.to_vec();
            v.insert(j, new);
            out.push(v);
        }
        out
    }

    /// Two people's edits of one list, applied in either order, give the same list.
    #[test]
    fn concurrent_single_edits_commute() {
        let base = [1, 2, 3, 4];
        for x in single_edits(&base, 10) {
            for y in single_edits(&base, 20) {
                let a = diff(&base, &x).unwrap();
                let b = diff(&base, &y).unwrap();
                // In the log `a` then `b`: every copy ends with `a`, then `b` rebased past it…
                let one = run(&run(&base, &a), &rebase(&b, &a, false));
                // …which is what `b`'s author gets from `b` then `a` rebased past it.
                let two = run(&run(&base, &b), &rebase(&a, &b, true));
                assert_eq!(one, two, "a {x:?}, b {y:?}");
            }
        }
    }
}
