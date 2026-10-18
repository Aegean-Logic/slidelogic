//! The text of one shape as a flat sequence, so two people typing in the same text box both keep
//! what they typed (ported from WordLogic's stories, `../wordcraft/crates/engine/src/collab/`).
//!
//! The paragraphs read as units: each character with its run's formatting, each line break,
//! field or equation as one unit, and a mark ending every paragraph (the last one too) that
//! carries the paragraph's settings. An edit is a [`Delta`] over those units: keep, insert,
//! delete, patch the formatting of what's kept, patch a paragraph mark. Formatting travels as
//! JSON maps of only the keys that changed, so one person bolding and another colouring the same
//! words both keep theirs; the same key, the later one wins.
//!
//! Applying rebuilds only the paragraphs an edit reaches (normalised: neighbouring runs with the
//! same formatting joined); the others stay as they were. Where that can't give back exactly what
//! the author has (runs the model keeps apart though they look alike), [`super::capture`] sends
//! the shape whole instead.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use deckcraft_model::text::{ParaProps, Paragraph, Run, RunKind, RunProps};

use super::refused;
use crate::Result;

pub type Map = serde_json::Map<String, Json>;

/// The most units a shape's text may grow to through edits.
const MAX_UNITS: usize = 4 << 20;

/// Inserted content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Item {
    /// Characters sharing one formatting (`p`, the run's formatting as JSON).
    #[serde(rename = "s")]
    Text {
        s: String,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        p: Map,
    },
    /// A line break, field or equation: the whole run.
    #[serde(rename = "o")]
    Atom { o: Json },
    /// A paragraph mark: [`mark_map`].
    #[serde(rename = "e")]
    End {
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        a: Map,
    },
}

impl Item {
    pub fn len(&self) -> usize {
        match self {
            Item::Text { s, .. } => s.chars().count(),
            _ => 1,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One step of a [`Delta`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k")]
pub enum Comp {
    /// Keep `n` units.
    #[serde(rename = "r")]
    Retain { n: usize },
    #[serde(rename = "i")]
    Insert { items: Vec<Item> },
    #[serde(rename = "d")]
    Delete { n: usize },
    /// Patch the run formatting of `n` characters (or breaks, fields, equations).
    #[serde(rename = "f")]
    Format { n: usize, p: Map },
    /// Patch a paragraph mark's settings ([`mark_map`] keys).
    #[serde(rename = "p")]
    Para { p: Map },
}

impl Comp {
    /// Units of the old sequence it covers (0 for an insert).
    fn input_len(&self) -> usize {
        match self {
            Comp::Retain { n } | Comp::Delete { n } | Comp::Format { n, .. } => *n,
            Comp::Insert { .. } => 0,
            Comp::Para { .. } => 1,
        }
    }
    /// Units of the new sequence it makes (0 for a delete).
    fn output_len(&self) -> usize {
        match self {
            Comp::Retain { n } | Comp::Format { n, .. } => *n,
            Comp::Insert { items } => items.iter().map(Item::len).sum(),
            Comp::Delete { .. } => 0,
            Comp::Para { .. } => 1,
        }
    }
    fn is_insert(&self) -> bool {
        matches!(self, Comp::Insert { .. })
    }
}

/// The steps of an edit to one shape's text. A trailing keep is implied.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Delta(pub Vec<Comp>);

impl Delta {
    pub fn is_noop(&self) -> bool {
        self.0.iter().all(|c| matches!(c, Comp::Retain { .. }))
    }

    /// Steps it has (for the size cap).
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Where the first change is in the old sequence, and the end of what the changes read.
    fn span(&self) -> Option<(usize, usize)> {
        let mut at = 0usize;
        let mut start = None;
        let mut end = 0usize;
        for c in &self.0 {
            if let Comp::Retain { n } = c {
                at = at.saturating_add(*n);
                continue;
            }
            start.get_or_insert(at);
            at = at.saturating_add(c.input_len());
            end = at;
        }
        start.map(|s| (s, end.max(s)))
    }

    /// Where `at`, a unit position before the edit, is after it. `stay` keeps it in front of an
    /// insert made right there (someone else's text typed at this person's caret goes after it).
    pub fn map_pos(&self, at: usize, stay: bool) -> usize {
        let mut old = 0usize;
        let mut new = 0usize;
        for c in &self.0 {
            match c {
                Comp::Insert { .. } => {
                    if old < at || (old == at && !stay) {
                        new = new.saturating_add(c.output_len());
                    }
                }
                Comp::Delete { n } => {
                    if old.saturating_add(*n) > at {
                        return new;
                    }
                    old = old.saturating_add(*n);
                }
                _ => {
                    let n = c.input_len();
                    if old.saturating_add(n) > at {
                        return new.saturating_add(at.saturating_sub(old));
                    }
                    old = old.saturating_add(n);
                    new = new.saturating_add(n);
                }
            }
        }
        new.saturating_add(at.saturating_sub(old))
    }
}

/// Appends steps, merging neighbours of the same kind and dropping empty ones.
#[derive(Default)]
struct Builder(Vec<Comp>);

impl Builder {
    fn retain(&mut self, n: usize) {
        self.push(Comp::Retain { n });
    }
    fn delete(&mut self, n: usize) {
        self.push(Comp::Delete { n });
    }

    fn push(&mut self, c: Comp) {
        match &c {
            Comp::Retain { n } | Comp::Delete { n } | Comp::Format { n, .. } if *n == 0 => return,
            Comp::Insert { items } if items.is_empty() => return,
            Comp::Format { p, n } if p.is_empty() => return self.retain(*n),
            Comp::Para { p } if p.is_empty() => return self.retain(1),
            _ => {}
        }
        // An insert goes before a delete at the same place (the same result, one shape).
        if c.is_insert()
            && let Some(Comp::Delete { n }) = self.0.last()
        {
            let n = *n;
            self.0.pop();
            self.push(c);
            self.delete(n);
            return;
        }
        match (self.0.last_mut(), c) {
            (Some(Comp::Retain { n: a }), Comp::Retain { n: b }) => *a = a.saturating_add(b),
            (Some(Comp::Delete { n: a }), Comp::Delete { n: b }) => *a = a.saturating_add(b),
            (Some(Comp::Format { n: a, p: pa }), Comp::Format { n: b, p: pb }) if *pa == pb => *a = a.saturating_add(b),
            (Some(Comp::Insert { items: a }), Comp::Insert { items: b }) => {
                for it in b {
                    push_item(a, it);
                }
            }
            (_, c) => self.0.push(c),
        }
    }

    /// The steps, without a trailing keep.
    fn done(mut self) -> Delta {
        while matches!(self.0.last(), Some(Comp::Retain { .. })) {
            self.0.pop();
        }
        Delta(self.0)
    }
}

/// Appends an item, joining text to text with the same formatting.
fn push_item(items: &mut Vec<Item>, it: Item) {
    if let Item::Text { s, p } = &it {
        if s.is_empty() {
            return;
        }
        if let Some(Item::Text { s: s0, p: p0 }) = items.last_mut()
            && p0 == p
        {
            s0.push_str(s);
            return;
        }
    }
    items.push(it);
}

/// Splits items after `n` units.
fn split_items(items: Vec<Item>, n: usize) -> (Vec<Item>, Vec<Item>) {
    let mut head = Vec::new();
    let mut tail = Vec::new();
    let mut left = n;
    for it in items {
        let len = it.len();
        if left == 0 {
            tail.push(it);
        } else if len <= left {
            left -= len;
            head.push(it);
        } else if let Item::Text { s, p } = it {
            let cut = s.char_indices().nth(left).map_or(s.len(), |(i, _)| i);
            let (a, b) = s.split_at(cut);
            head.push(Item::Text { s: a.to_string(), p: p.clone() });
            tail.push(Item::Text { s: b.to_string(), p });
            left = 0;
        } else {
            tail.push(it);
        }
    }
    (head, tail)
}

/// Walks a list of steps, handing out pieces of a given length.
struct Iter {
    comps: std::collections::VecDeque<Comp>,
}

impl Iter {
    fn new(comps: &[Comp]) -> Iter {
        // Empty steps would stall the walk: they change nothing anyway.
        Iter { comps: comps.iter().filter(|c| c.input_len() > 0 || c.output_len() > 0).cloned().collect() }
    }
    fn has(&self) -> bool {
        !self.comps.is_empty()
    }
    fn peek_insert(&self) -> bool {
        self.comps.front().is_some_and(Comp::is_insert)
    }
    fn peek_len(&self) -> usize {
        match self.comps.front() {
            Some(c) if c.is_insert() => c.output_len(),
            Some(c) => c.input_len(),
            None => usize::MAX,
        }
    }
    /// The next step, cut to at most `n` units (a keep once the steps are done).
    fn next(&mut self, n: usize) -> Comp {
        let Some(c) = self.comps.pop_front() else { return Comp::Retain { n } };
        let len = if c.is_insert() { c.output_len() } else { c.input_len() };
        if n >= len {
            return c;
        }
        let (head, tail) = match c {
            Comp::Retain { n: m } => (Comp::Retain { n }, Comp::Retain { n: m - n }),
            Comp::Delete { n: m } => (Comp::Delete { n }, Comp::Delete { n: m - n }),
            Comp::Format { n: m, p } => (Comp::Format { n, p: p.clone() }, Comp::Format { n: m - n, p }),
            Comp::Insert { items } => {
                let (a, b) = split_items(items, n);
                (Comp::Insert { items: a }, Comp::Insert { items: b })
            }
            // One unit long: never cut.
            other => return other,
        };
        self.comps.push_front(tail);
        head
    }
    fn next_all(&mut self) -> Comp {
        self.next(usize::MAX)
    }
}

/// `p` without the keys `q` sets.
fn minus_keys(p: &Map, q: &Map) -> Map {
    p.iter().filter(|(k, _)| !q.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Applies a shallow patch: a key set to `null` goes.
fn patch_map(m: &mut Map, p: &Map) {
    for (k, v) in p {
        if v.is_null() {
            m.remove(k);
        } else {
            m.insert(k.clone(), v.clone());
        }
    }
}

/// The patch that turns `old` into `new`.
fn diff_map(old: &Map, new: &Map) -> Map {
    let mut p = Map::new();
    for (k, v) in new {
        if old.get(k) != Some(v) {
            p.insert(k.clone(), v.clone());
        }
    }
    for k in old.keys() {
        if !new.contains_key(k) {
            p.insert(k.clone(), Json::Null);
        }
    }
    p
}

/// `a` rebased to apply after `b`, both made against the same text. `a_first`: `a` comes first
/// in the log, so where both change the same thing `b` wins, and where both insert at one place
/// `a`'s text goes first.
pub fn transform(a: &Delta, b: &Delta, a_first: bool) -> Delta {
    let mut ai = Iter::new(&a.0);
    let mut bi = Iter::new(&b.0);
    let mut out = Builder::default();
    while ai.has() {
        if ai.peek_insert() && (a_first || !bi.peek_insert()) {
            out.push(ai.next_all());
            continue;
        }
        if bi.peek_insert() {
            out.retain(bi.next_all().output_len());
            continue;
        }
        let n = ai.peek_len().min(bi.peek_len());
        let ac = ai.next(n);
        let bc = bi.next(n);
        match (ac, bc) {
            (_, Comp::Delete { .. }) => {}
            (Comp::Delete { n }, _) => out.delete(n),
            (Comp::Retain { n }, _) => out.retain(n),
            (Comp::Format { n, p }, Comp::Format { p: q, .. }) => out.push(Comp::Format { n, p: if a_first { minus_keys(&p, &q) } else { p } }),
            (Comp::Para { p }, Comp::Para { p: q }) => out.push(Comp::Para { p: if a_first { minus_keys(&p, &q) } else { p } }),
            (c, _) => out.push(c),
        }
    }
    out.done()
}

// --- The model's pieces as JSON --------------------------------------------------------------

/// A value as a JSON map, without its unset (`null`) keys.
fn to_map<T: Serialize>(v: &T) -> Result<Map> {
    match serde_json::to_value(v).map_err(refused)? {
        Json::Object(m) => Ok(m.into_iter().filter(|(_, v)| !v.is_null()).collect()),
        _ => Ok(Map::new()),
    }
}

fn from_map<T: serde::de::DeserializeOwned>(m: Map) -> Result<T> {
    T::deserialize(Json::Object(m)).map_err(|e| refused(format!("bad formatting in a text edit: {e}")))
}

/// A paragraph's own settings, as its mark carries them.
#[derive(Clone, Debug, Default, PartialEq)]
struct Meta {
    level: u8,
    props: ParaProps,
    def_props: RunProps,
    end_props: RunProps,
}

impl Meta {
    fn of(p: &Paragraph) -> Meta {
        Meta { level: p.level, props: p.props.clone(), def_props: p.def_props.clone(), end_props: p.end_props.clone() }
    }
}

/// A mark's settings as one flat map: `level`, then `p.<key>` for the paragraph's formatting,
/// `d.<key>` for its run defaults and `e.<key>` for the end mark's formatting, so a patch changes
/// one setting without touching the others.
fn mark_map(m: &Meta) -> Result<Map> {
    let mut out = Map::new();
    if m.level != 0 {
        out.insert("level".into(), Json::from(m.level));
    }
    for (prefix, map) in [("p.", to_map(&m.props)?), ("d.", to_map(&m.def_props)?), ("e.", to_map(&m.end_props)?)] {
        for (k, v) in map {
            out.insert(format!("{prefix}{k}"), v);
        }
    }
    Ok(out)
}

fn mark_from_map(m: Map) -> Result<Meta> {
    let mut parts = [Map::new(), Map::new(), Map::new()];
    let mut level = 0u8;
    for (k, v) in m {
        if k == "level" {
            level = v.as_u64().and_then(|l| u8::try_from(l).ok()).filter(|l| *l <= 8).ok_or_else(|| refused("bad paragraph level in a text edit"))?;
        } else if let Some((i, key)) = ["p.", "d.", "e."].iter().enumerate().find_map(|(i, p)| k.strip_prefix(p).map(|key| (i, key)))
            && let Some(part) = parts.get_mut(i)
        {
            part.insert(key.to_string(), v);
        }
    }
    let [p, d, e] = parts;
    Ok(Meta { level, props: from_map(p)?, def_props: from_map(d)?, end_props: from_map(e)? })
}

// --- Units -----------------------------------------------------------------------------------

/// One unit of a paragraph list, borrowed.
#[derive(Clone, Copy)]
enum Unit<'a> {
    Char(char, &'a RunProps),
    Atom(&'a Run),
    Mark(&'a Paragraph),
}

impl Unit<'_> {
    /// The same content (formatting aside).
    fn same_content(&self, o: &Unit) -> bool {
        match (self, o) {
            (Unit::Char(a, _), Unit::Char(b, _)) => a == b,
            (Unit::Atom(a), Unit::Atom(b)) => a.kind == b.kind && a.text == b.text,
            (Unit::Mark(_), Unit::Mark(_)) => true,
            _ => false,
        }
    }
}

fn units(paras: &[Paragraph]) -> Vec<Unit<'_>> {
    let mut out = Vec::new();
    for p in paras {
        for r in &p.runs {
            match r.kind {
                RunKind::Text => out.extend(r.text.chars().map(|c| Unit::Char(c, &r.props))),
                _ => out.push(Unit::Atom(r)),
            }
        }
        out.push(Unit::Mark(p));
    }
    out
}

/// Units of a paragraph: its characters and atoms, and its mark.
fn para_len(p: &Paragraph) -> usize {
    p.runs.iter().map(|r| if r.kind == RunKind::Text { r.text.chars().count() } else { 1 }).sum::<usize>() + 1
}

fn unit_item(u: &Unit) -> Result<Item> {
    Ok(match u {
        Unit::Char(c, p) => Item::Text { s: c.to_string(), p: to_map(*p)? },
        Unit::Atom(r) => Item::Atom { o: serde_json::to_value(*r).map_err(refused)? },
        Unit::Mark(p) => Item::End { a: mark_map(&Meta::of(p))? },
    })
}

/// The edit that turns the paragraphs `old` into `new`.
pub fn diff(old: &[Paragraph], new: &[Paragraph]) -> Result<Delta> {
    // Whole paragraphs that stayed, at either end, are kept without looking inside.
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old.get(head..).unwrap_or(&[]).iter().rev().zip(new.get(head..).unwrap_or(&[]).iter().rev()).take_while(|(a, b)| a == b).count();
    let old_mid = old.get(head..old.len().saturating_sub(tail)).unwrap_or(&[]);
    let new_mid = new.get(head..new.len().saturating_sub(tail)).unwrap_or(&[]);
    let mut b = Builder::default();
    b.retain(old.get(..head).unwrap_or(&[]).iter().map(para_len).sum());
    let ou = units(old_mid);
    let nu = units(new_mid);
    let pre = ou.iter().zip(&nu).take_while(|(a, b)| a.same_content(b)).count();
    let max_suf = ou.len().min(nu.len()) - pre;
    let suf = ou.iter().rev().zip(nu.iter().rev()).take(max_suf).take_while(|(a, b)| a.same_content(b)).count();
    keep(&mut b, ou.get(..pre).unwrap_or(&[]), nu.get(..pre).unwrap_or(&[]))?;
    b.delete(ou.len() - pre - suf);
    let mut items = Vec::new();
    for u in nu.get(pre..nu.len() - suf).unwrap_or(&[]) {
        push_item(&mut items, unit_item(u)?);
    }
    b.push(Comp::Insert { items });
    keep(&mut b, ou.get(ou.len() - suf..).unwrap_or(&[]), nu.get(nu.len() - suf..).unwrap_or(&[]))?;
    Ok(b.done())
}

/// Keeps units whose content stayed, patching what changed of their formatting.
fn keep(b: &mut Builder, old: &[Unit], new: &[Unit]) -> Result<()> {
    for (o, n) in old.iter().zip(new) {
        match (o, n) {
            (Unit::Char(_, a), Unit::Char(_, c)) if a == c => b.retain(1),
            (Unit::Atom(a), Unit::Atom(c)) if a.props == c.props => b.retain(1),
            (Unit::Char(_, a), Unit::Char(_, c)) => b.push(Comp::Format { n: 1, p: diff_map(&to_map(*a)?, &to_map(*c)?) }),
            (Unit::Atom(a), Unit::Atom(c)) => b.push(Comp::Format { n: 1, p: diff_map(&to_map(&a.props)?, &to_map(&c.props)?) }),
            (Unit::Mark(a), Unit::Mark(c)) => {
                let (ma, mc) = (Meta::of(a), Meta::of(c));
                if ma == mc {
                    b.retain(1);
                } else {
                    b.push(Comp::Para { p: diff_map(&mark_map(&ma)?, &mark_map(&mc)?) });
                }
            }
            // `keep` gets units of the same content only.
            _ => return Err(refused("text units out of step")),
        }
    }
    Ok(())
}

// --- Applying --------------------------------------------------------------------------------

/// A piece of the sequence, owned.
#[derive(Clone, Debug)]
enum Seg {
    Text { text: String, props: RunProps },
    Atom(Run),
    Mark(Meta),
}

impl Seg {
    fn len(&self) -> usize {
        match self {
            Seg::Text { text, .. } => text.chars().count(),
            _ => 1,
        }
    }
}

fn segs(paras: &[Paragraph]) -> Vec<Seg> {
    let mut out = Vec::new();
    for p in paras {
        for r in &p.runs {
            match r.kind {
                RunKind::Text if r.text.is_empty() => {}
                RunKind::Text => out.push(Seg::Text { text: r.text.clone(), props: r.props.clone() }),
                _ => out.push(Seg::Atom(r.clone())),
            }
        }
        out.push(Seg::Mark(Meta::of(p)));
    }
    out
}

/// Hands out the old sequence's segments `n` units at a time.
struct Cursor {
    segs: std::collections::VecDeque<Seg>,
}

impl Cursor {
    fn take(&mut self, mut n: usize) -> Option<Vec<Seg>> {
        let mut out = Vec::new();
        while n > 0 {
            let seg = self.segs.pop_front()?;
            let len = seg.len();
            if len <= n {
                n -= len;
                out.push(seg);
            } else if let Seg::Text { text, props } = seg {
                let cut = text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
                let (a, b) = text.split_at(cut);
                out.push(Seg::Text { text: a.to_string(), props: props.clone() });
                self.segs.push_front(Seg::Text { text: b.to_string(), props });
                n = 0;
            } else {
                return None;
            }
        }
        Some(out)
    }
}

fn patched(props: &RunProps, p: &Map) -> Result<RunProps> {
    let mut m = to_map(props)?;
    patch_map(&mut m, p);
    from_map(m)
}

fn item_seg(it: &Item) -> Result<Seg> {
    Ok(match it {
        Item::Text { s, p } => {
            if s.is_empty() {
                return Err(refused("empty text in a text edit"));
            }
            Seg::Text { text: s.clone(), props: from_map(p.clone())? }
        }
        Item::Atom { o } => {
            let run = Run::deserialize(o).map_err(|e| refused(format!("bad run in a text edit: {e}")))?;
            if run.kind == RunKind::Text {
                return Err(refused("a text run where a break, field or equation belongs"));
            }
            Seg::Atom(run)
        }
        Item::End { a } => Seg::Mark(mark_from_map(a.clone())?),
    })
}

/// Applies `comps` to `segs` (anything after the steps is kept).
fn apply_segs(segs: Vec<Seg>, comps: &[Comp]) -> Result<Vec<Seg>> {
    let short = || refused("a text edit runs past the end of the text");
    let mut src = Cursor { segs: segs.into() };
    let mut out = Vec::new();
    for c in comps {
        match c {
            Comp::Retain { n } => out.extend(src.take(*n).ok_or_else(short)?),
            Comp::Insert { items } => {
                for it in items {
                    out.push(item_seg(it)?);
                }
            }
            Comp::Delete { n } => {
                src.take(*n).ok_or_else(short)?;
            }
            Comp::Format { n, p } => {
                for seg in src.take(*n).ok_or_else(short)? {
                    out.push(match seg {
                        Seg::Text { text, props } => Seg::Text { text, props: patched(&props, p)? },
                        Seg::Atom(mut run) => {
                            run.props = patched(&run.props, p)?;
                            Seg::Atom(run)
                        }
                        Seg::Mark(_) => return Err(refused("run formatting for a paragraph mark")),
                    });
                }
            }
            Comp::Para { p } => match src.take(1).ok_or_else(short)?.pop() {
                Some(Seg::Mark(m)) => {
                    let mut map = mark_map(&m)?;
                    patch_map(&mut map, p);
                    out.push(Seg::Mark(mark_from_map(map)?));
                }
                _ => return Err(refused("paragraph settings for something not a paragraph mark")),
            },
        }
    }
    out.extend(src.segs);
    Ok(out)
}

/// Paragraphs from segments, each normalised. Units after the last mark make a paragraph of
/// their own.
fn paragraphs(segs: Vec<Seg>) -> Vec<Paragraph> {
    let mut out = Vec::new();
    let mut runs: Vec<Run> = Vec::new();
    let mut open = false;
    for seg in segs {
        match seg {
            Seg::Text { text, props } => {
                runs.push(Run { text, props, kind: RunKind::Text });
                open = true;
            }
            Seg::Atom(run) => {
                runs.push(run);
                open = true;
            }
            Seg::Mark(m) => {
                let mut p =
                    Paragraph { level: m.level, props: m.props, runs: std::mem::take(&mut runs), def_props: m.def_props, end_props: m.end_props };
                p.normalize();
                out.push(p);
                open = false;
            }
        }
    }
    if open {
        let mut p = Paragraph { runs, ..Default::default() };
        p.normalize();
        out.push(p);
    }
    out
}

/// Applies `delta` to `paras`, rebuilding only the paragraphs it reaches.
pub fn apply(paras: &mut Vec<Paragraph>, delta: &Delta) -> Result<()> {
    let Some((start, end)) = delta.span() else { return Ok(()) };
    let total: usize = paras.iter().map(para_len).sum();
    if start > total {
        return Err(refused("a text edit starts past the end of the text"));
    }
    let added: usize = delta
        .0
        .iter()
        .map(Comp::output_len)
        .sum::<usize>()
        .saturating_sub(delta.0.iter().map(|c| if c.is_insert() { 0 } else { c.input_len() }).sum());
    if total.saturating_add(added) > MAX_UNITS {
        return Err(refused("the text would grow too large"));
    }
    // The paragraphs the changes reach: from the one holding `start` to the one holding the unit
    // right after what they read (a paragraph mark deleted joins the next paragraph in), or
    // `start` for an insert; an insert at the very end reaches the last one.
    let mut first = None;
    let mut last = None;
    let mut at = 0usize;
    let last_unit = end.max(start);
    for (i, p) in paras.iter().enumerate() {
        let len = para_len(p);
        if first.is_none() && start < at + len {
            first = Some((i, at));
        }
        if last_unit < at + len {
            last = Some(i);
            break;
        }
        at += len;
    }
    let n = paras.len();
    let (first, lead) = first.unwrap_or((n.saturating_sub(1), total - paras.last().map_or(0, para_len)));
    let last = last.unwrap_or(n.saturating_sub(1)).max(first);
    let reached = paras.get(first..=last).unwrap_or(&[]);
    // The steps, from the first reached paragraph on.
    let mut comps = delta.0.clone();
    match comps.first_mut() {
        Some(Comp::Retain { n }) if *n >= lead => *n -= lead,
        _ if lead == 0 => {}
        _ => return Err(refused("a text edit out of step with the text")),
    }
    let rebuilt = paragraphs(apply_segs(segs(reached), &comps)?);
    let tail: Vec<Paragraph> = paras.get(last + 1..).unwrap_or(&[]).to_vec();
    paras.truncate(first.min(paras.len()));
    paras.extend(rebuilt);
    paras.extend(tail);
    Ok(())
}

// --- Positions -------------------------------------------------------------------------------

/// The unit position of (paragraph, char) in `paras`.
pub fn flat_pos(paras: &[Paragraph], (para, ch): (usize, usize)) -> usize {
    let before: usize = paras.get(..para).unwrap_or(paras).iter().map(para_len).sum();
    let len = paras.get(para).map_or(0, |p| para_len(p) - 1);
    before + ch.min(len)
}

/// (paragraph, char) of the unit position `at` in `paras`.
pub fn pos_at(paras: &[Paragraph], mut at: usize) -> (usize, usize) {
    for (i, p) in paras.iter().enumerate() {
        let len = para_len(p);
        if at < len {
            return (i, at);
        }
        at -= len;
    }
    let last = paras.len().saturating_sub(1);
    (last, paras.get(last).map_or(0, |p| para_len(p) - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paras(lines: &[&str]) -> Vec<Paragraph> {
        lines.iter().map(|l| Paragraph::new(l)).collect()
    }

    fn text(p: &[Paragraph]) -> Vec<String> {
        p.iter().map(Paragraph::text).collect()
    }

    fn edited(base: &[Paragraph], f: impl FnOnce(&mut Vec<Paragraph>)) -> Vec<Paragraph> {
        let mut p = base.to_vec();
        f(&mut p);
        for q in p.iter_mut() {
            q.normalize();
        }
        p
    }

    fn run(base: &[Paragraph], d: &Delta) -> Vec<Paragraph> {
        let mut p = base.to_vec();
        apply(&mut p, d).unwrap();
        p
    }

    #[test]
    fn diff_then_apply_rebuilds_the_text() {
        let base = paras(&["Hello world", "Second line", "Third"]);
        let bold = RunProps { bold: Some(true), ..Default::default() };
        let cases: Vec<Vec<Paragraph>> = vec![
            edited(&base, |p| p[1] = Paragraph::new("Second shiny line")),
            edited(&base, |p| {
                p.remove(1);
            }),
            edited(&base, |p| p.insert(0, Paragraph::new("New first"))),
            edited(&base, |p| {
                p[0].runs = vec![Run::new("Hello "), Run::with("world", bold.clone())];
            }),
            edited(&base, |p| p[2].level = 2),
            edited(&base, |p| p[0].runs.push(Run { text: String::new(), props: RunProps::default(), kind: RunKind::Break })),
            vec![],
            paras(&["Completely different"]),
        ];
        for new in cases {
            let d = diff(&base, &new).unwrap();
            assert_eq!(run(&base, &d), new, "{d:?}");
            let back = diff(&new, &base).unwrap();
            assert_eq!(run(&new, &back), base);
            // As it travels.
            let json = serde_json::to_string(&d).unwrap();
            assert_eq!(serde_json::from_str::<Delta>(&json).unwrap(), d);
        }
        assert!(diff(&base, &base).unwrap().is_noop());
    }

    #[test]
    fn typing_in_one_paragraph_touches_only_that_one() {
        let base = paras(&["One", "Two", "Three"]);
        let new = edited(&base, |p| p[1] = Paragraph::new("Two and a half"));
        let d = diff(&base, &new).unwrap();
        assert_eq!(d.0.first(), Some(&Comp::Retain { n: 4 + 3 }), "kept: One¶ and Two");
    }

    /// Two people's edits of one text, applied in either order, give the same text.
    #[test]
    fn concurrent_edits_commute() {
        let base = paras(&["The quick fox", "jumps over"]);
        let bold = RunProps { bold: Some(true), ..Default::default() };
        let italic = RunProps { italic: Some(true), ..Default::default() };
        let edits: Vec<Vec<Paragraph>> = vec![
            edited(&base, |p| p[0] = Paragraph::new("The quick brown fox")),
            edited(&base, |p| p[0] = Paragraph::new("The fox")),
            edited(&base, |p| p[1] = Paragraph::new("jumps right over")),
            edited(&base, |p| p[0] = Paragraph::new("A quick fox")),
            edited(&base, |p| p[0].runs = vec![Run::new("The "), Run::with("quick", bold.clone()), Run::new(" fox")]),
            edited(&base, |p| p[0].runs = vec![Run::new("The "), Run::with("quick", italic.clone()), Run::new(" fox")]),
            edited(&base, |p| {
                let second = p.remove(1);
                p[0].runs.extend(second.runs);
            }),
            edited(&base, |p| p.insert(1, Paragraph::new("Middle"))),
            edited(&base, |p| p[1].level = 1),
        ];
        for x in &edits {
            for y in &edits {
                let a = diff(&base, x).unwrap();
                let b = diff(&base, y).unwrap();
                let one = run(&run(&base, &a), &transform(&b, &a, false));
                let two = run(&run(&base, &b), &transform(&a, &b, true));
                assert_eq!(text(&one), text(&two), "a {:?}, b {:?}", text(x), text(y));
                assert_eq!(one, two, "a {:?}, b {:?}", text(x), text(y));
            }
        }
    }

    #[test]
    fn both_typed_texts_stay() {
        let base = paras(&["Hello"]);
        let a = diff(&base, &paras(&["Hello Ann"])).unwrap();
        let b = diff(&base, &paras(&["Bob: Hello"])).unwrap();
        let merged = run(&run(&base, &a), &transform(&b, &a, false));
        assert_eq!(text(&merged), vec!["Bob: Hello Ann"]);
        // At the same place: the earlier in the log first.
        let a = diff(&base, &paras(&["Hello A"])).unwrap();
        let b = diff(&base, &paras(&["Hello B"])).unwrap();
        let merged = run(&run(&base, &a), &transform(&b, &a, false));
        assert_eq!(text(&merged), vec!["Hello A B"]);
    }

    #[test]
    fn bold_and_italic_of_the_same_words_both_stay() {
        let base = paras(&["word"]);
        let bold = edited(&base, |p| p[0].runs[0].props.bold = Some(true));
        let italic = edited(&base, |p| p[0].runs[0].props.italic = Some(true));
        let a = diff(&base, &bold).unwrap();
        let b = diff(&base, &italic).unwrap();
        let merged = run(&run(&base, &a), &transform(&b, &a, false));
        assert_eq!(merged[0].runs[0].props.bold, Some(true));
        assert_eq!(merged[0].runs[0].props.italic, Some(true));
    }

    #[test]
    fn positions_follow_the_edit() {
        let base = paras(&["Hello", "world"]);
        let new = paras(&["Hi, Hello", "world"]);
        let d = diff(&base, &new).unwrap();
        // The caret after "world"'s "w" moves with its text; one in front of the insert stays.
        let at = d.map_pos(flat_pos(&base, (1, 1)), true);
        assert_eq!(pos_at(&new, at), (1, 1));
        assert_eq!(pos_at(&new, d.map_pos(flat_pos(&base, (0, 0)), true)), (0, 0));
        assert_eq!(pos_at(&new, d.map_pos(flat_pos(&base, (0, 5)), true)), (0, 9));
    }

    #[test]
    fn hostile_edits_are_refused() {
        let base = paras(&["Hi"]);
        let bad = |comps: Vec<Comp>| {
            let mut p = base.clone();
            apply(&mut p, &Delta(comps)).is_err()
        };
        assert!(bad(vec![Comp::Retain { n: 50 }, Comp::Delete { n: 1 }]), "past the end");
        assert!(bad(vec![Comp::Delete { n: 9 }]));
        assert!(
            bad(vec![Comp::Retain { n: 2 }, Comp::Format { n: 1, p: [("bold".to_string(), Json::from(true))].into_iter().collect() }]),
            "run formatting for a mark"
        );
        assert!(bad(vec![Comp::Para { p: [("level".to_string(), Json::from(99))].into_iter().collect() }]), "a run is not a mark");
        assert!(bad(vec![Comp::Insert { items: vec![Item::Atom { o: serde_json::json!({"text": "x"}) }] }]), "a text run as an atom");
        assert!(bad(vec![Comp::Insert { items: vec![Item::Text { s: "x".repeat(MAX_UNITS + 1), p: Map::new() }] }]), "too large");
        assert!(
            bad(vec![Comp::Retain { n: 2 }, Comp::Para { p: [("level".to_string(), Json::from(99))].into_iter().collect() }]),
            "level out of range"
        );
    }
}
