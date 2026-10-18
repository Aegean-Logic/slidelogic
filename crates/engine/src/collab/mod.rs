//! Real-time co-editing (`docs/co-editing.md`).
//!
//! Copies of one presentation stay the same by exchanging an [`Op`] per edit: the result of a
//! local command, diffed from the presentation before and after it ([`capture`]), so every
//! command is shared without code of its own.
//! - Slides and masters: their order as [`SeqEdit`]s by id, the only part that has to be moved
//!   past someone else's edit ([`rebase`]); new ones travel whole.
//! - A changed slide: its own settings (layout, background, transition, notes…) one by one, and
//!   its shapes by [`ShapeId`]: added, removed, restacked or replaced whole (a group with
//!   everything in it).
//! - A changed master (with its layouts and theme) is replaced whole.
//! - Presentation settings (slide size, sections, show settings, properties…) one by one, each
//!   replaced whole.
//! - Media by [`MediaId`]: new or changed bytes travel with the op that first uses them.
//!
//! Where two people change the same thing at once, the later one in the log wins it whole.
//! Nothing here does I/O: the host moves ops between people ([`Replica`] is one person's copy).

mod replica;
mod seq;

pub use replica::{Incoming, Outgoing, Replica, Shown};
pub use seq::SeqEdit;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use deckcraft_model::{LayoutId, Master, MasterId, MediaId, MediaItem, Presentation, Shape, ShapeId, Slide, SlideId, walk};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::{EngineError, Result};

/// Settings by field name, each with its whole new value.
pub type Fields = BTreeMap<String, Json>;

/// The most edits, items and settings one op may carry.
pub const MAX_OP_ITEMS: usize = 100_000;
/// The most media bytes one op may carry.
pub const MAX_OP_MEDIA_BYTES: usize = 512 << 20;
/// The most work reordering one list may take (edits × items): a hostile op can't stall a copy.
const MAX_ORDER_WORK: usize = 500_000_000;

/// The shapes of one slide: their order, and the new and changed ones whole.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShapesPatch {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<SeqEdit<ShapeId>>,
    /// New shapes (an insert names them) and changed ones, top-level shapes whole.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub put: Vec<Shape>,
}

impl ShapesPatch {
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty() && self.put.is_empty()
    }
}

/// What changed on a slide both copies have.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SlideChange {
    pub id: SlideId,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: Fields,
    #[serde(skip_serializing_if = "ShapesPatch::is_empty")]
    pub shapes: ShapesPatch,
}

impl SlideChange {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.shapes.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SlidesPatch {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<SeqEdit<SlideId>>,
    /// New slides (an insert names them), whole.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<Arc<Slide>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<SlideChange>,
}

impl SlidesPatch {
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty() && self.added.is_empty() && self.changed.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MastersPatch {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<SeqEdit<MasterId>>,
    /// New and changed masters, whole.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub put: Vec<Arc<Master>>,
}

impl MastersPatch {
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty() && self.put.is_empty()
    }
}

/// Media bytes, as base64 when the op travels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bytes(pub Arc<Vec<u8>>);

impl Serialize for Bytes {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&crate::cmd::base64_encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;
        let s = String::deserialize(d)?;
        if s.len() / 4 * 3 > MAX_OP_MEDIA_BYTES {
            return Err(D::Error::custom("co-editing: media too large"));
        }
        crate::cmd::base64_decode(&s).map(|v| Bytes(Arc::new(v))).ok_or_else(|| D::Error::custom("co-editing: damaged media data"))
    }
}

/// A new or changed media item.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaPut {
    pub id: MediaId,
    pub name: String,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    pub data: Bytes,
}

/// One edit as the others receive it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Op {
    /// Presentation settings.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: Fields,
    #[serde(skip_serializing_if = "MastersPatch::is_empty")]
    pub masters: MastersPatch,
    #[serde(skip_serializing_if = "SlidesPatch::is_empty")]
    pub slides: SlidesPatch,
    /// Applied first, so everything after can use it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<MediaPut>,
    /// Applied last, and only where nothing uses the item any more (someone else may have just
    /// put the same picture in).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub media_removed: Vec<MediaId>,
}

impl Op {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.masters.is_empty() && self.slides.is_empty() && self.media.is_empty() && self.media_removed.is_empty()
    }

    /// Edits, items and settings in the op, for the size cap.
    fn items(&self) -> usize {
        let shapes: usize = self.slides.changed.iter().map(|c| c.fields.len() + c.shapes.edits.len() + c.shapes.put.len()).sum();
        self.fields.len()
            + self.masters.edits.len()
            + self.masters.put.len()
            + self.slides.edits.len()
            + self.slides.added.len()
            + self.slides.changed.len()
            + shapes
            + self.media.len()
            + self.media_removed.len()
    }
}

fn refused(msg: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("co-editing: {msg}"))
}

fn to_json<T: Serialize>(v: &T) -> Result<Json> {
    serde_json::to_value(v).map_err(refused)
}

fn from_json<T: serde::de::DeserializeOwned>(v: &Json) -> Result<T> {
    T::deserialize(v).map_err(refused)
}

/// The same value as far as the others can tell: equal, or equal once written out (a NaN isn't
/// equal to itself, and travels as `null` anyway).
fn same<T: PartialEq + Serialize>(a: &T, b: &T) -> bool {
    a == b || matches!((serde_json::to_value(a), serde_json::to_value(b)), (Ok(x), Ok(y)) if x == y)
}

/// The diff and apply functions for one struct's settings. The struct is destructured in full,
/// so a field added to the model doesn't compile until it's listed here: shared, or handled
/// elsewhere.
macro_rules! settings {
    ($diff:ident, $apply:ident, $ty:ident, [$($f:ident),* $(,)?], elsewhere: [$($e:ident),* $(,)?]) => {
        /// The settings of `b` that differ from `a`'s.
        fn $diff(a: &$ty, b: &$ty) -> Result<Fields> {
            let $ty { $($f: _,)* $($e: _,)* } = b;
            let mut out = Fields::new();
            $(
                if !same(&a.$f, &b.$f) {
                    out.insert(stringify!($f).to_string(), to_json(&b.$f)?);
                }
            )*
            Ok(out)
        }

        fn $apply(t: &mut $ty, fields: &Fields) -> Result<()> {
            for (k, v) in fields {
                match k.as_str() {
                    $(stringify!($f) => t.$f = from_json(v)?,)*
                    _ => return Err(refused(format!("unknown setting `{k}`"))),
                }
            }
            Ok(())
        }
    };
}

settings!(
    slide_settings,
    apply_slide_settings,
    Slide,
    [layout, name, background, hidden, show_master_shapes, transition, animations, notes, comments, color_map, tags, raw_ext],
    elsewhere: [id, shapes]
);

settings!(
    presentation_settings,
    apply_presentation_settings,
    Presentation,
    [
        slide_size,
        notes_size,
        notes_master,
        handout_master,
        sections,
        custom_shows,
        show,
        header_footer,
        props,
        default_text_style,
        first_slide_number,
        embedded_fonts,
        table_styles,
        tags,
        kept_parts,
        guides,
        raw_ext,
    ],
    // `next_id` is each copy's own: new ids come from its own range (`id_space`).
    elsewhere: [slides, masters, media, next_id]
);

/// The edit that turned `before` into `after`. Fails only on a list that repeats an id.
pub fn capture(before: &Presentation, after: &Presentation) -> Result<Op> {
    let mut op = Op { fields: presentation_settings(before, after)?, ..Op::default() };

    let master_ids = |v: &[Arc<Master>]| v.iter().map(|m| m.id).collect::<Vec<_>>();
    op.masters.edits = seq::diff(&master_ids(&before.masters), &master_ids(&after.masters)).ok_or_else(|| refused("two masters share an id"))?;
    let old: HashMap<MasterId, &Arc<Master>> = before.masters.iter().map(|m| (m.id, m)).collect();
    for m in &after.masters {
        match old.get(&m.id) {
            Some(o) if Arc::ptr_eq(o, m) || same(&***o, &**m) => {}
            _ => op.masters.put.push(m.clone()),
        }
    }

    let slide_ids = |v: &[Arc<Slide>]| v.iter().map(|s| s.id).collect::<Vec<_>>();
    op.slides.edits = seq::diff(&slide_ids(&before.slides), &slide_ids(&after.slides)).ok_or_else(|| refused("two slides share an id"))?;
    let old: HashMap<SlideId, &Arc<Slide>> = before.slides.iter().map(|s| (s.id, s)).collect();
    for s in &after.slides {
        match old.get(&s.id) {
            None => op.slides.added.push(s.clone()),
            Some(o) if Arc::ptr_eq(o, s) || ***o == **s => {}
            Some(o) => {
                let change = SlideChange { id: s.id, fields: slide_settings(o, s)?, shapes: capture_shapes(&o.shapes, &s.shapes)? };
                if !change.is_empty() {
                    op.slides.changed.push(change);
                }
            }
        }
    }

    let old: HashMap<MediaId, &MediaItem> = before.media.iter().map(|m| (m.id, m)).collect();
    for m in &after.media {
        let changed = old.get(&m.id).is_none_or(|o| {
            o.name != m.name || o.content_type != m.content_type || o.link != m.link || !(Arc::ptr_eq(&o.data, &m.data) || o.data == m.data)
        });
        if changed {
            op.media.push(MediaPut {
                id: m.id,
                name: m.name.clone(),
                content_type: m.content_type.clone(),
                link: m.link.clone(),
                data: Bytes(m.data.clone()),
            });
        }
    }
    let now: HashSet<MediaId> = after.media.iter().map(|m| m.id).collect();
    let mut gone = HashSet::new();
    op.media_removed = before.media.iter().map(|m| m.id).filter(|id| !now.contains(id) && gone.insert(*id)).collect();
    Ok(op)
}

fn capture_shapes(before: &[Shape], after: &[Shape]) -> Result<ShapesPatch> {
    let ids = |v: &[Shape]| v.iter().map(|s| s.id).collect::<Vec<_>>();
    let edits = seq::diff(&ids(before), &ids(after)).ok_or_else(|| refused("two shapes on a slide share an id"))?;
    let old: HashMap<ShapeId, &Shape> = before.iter().map(|s| (s.id, s)).collect();
    let put = after.iter().filter(|s| old.get(&s.id).is_none_or(|o| !same(*o, *s))).cloned().collect();
    Ok(ShapesPatch { edits, put })
}

/// Applies `op` to `p`. On an error `p` may be half changed: apply to a copy. Every copy refuses
/// the same ops, and treats what no longer fits (a change to a slide someone removed) the same
/// way, so copies that apply the same ops stay the same.
pub fn apply(p: &mut Presentation, op: &Op) -> Result<()> {
    if op.items() > MAX_OP_ITEMS {
        return Err(refused("the edit is too large to share"));
    }
    let bytes = op.media.iter().fold(0usize, |n, m| n.saturating_add(m.data.0.len()));
    if bytes > MAX_OP_MEDIA_BYTES {
        return Err(refused("the edit's pictures and media are too large to share"));
    }
    for m in &op.media {
        let item = MediaItem { id: m.id, name: m.name.clone(), content_type: m.content_type.clone(), data: m.data.0.clone(), link: m.link.clone() };
        match p.media.iter_mut().find(|x| x.id == m.id) {
            Some(x) => *x = item,
            None => p.media.push(item),
        }
    }
    apply_presentation_settings(p, &op.fields)?;

    order_work(op.masters.edits.len(), p.masters.len())?;
    let put = &op.masters.put;
    seq::apply(&mut p.masters, &op.masters.edits, |m| m.id, |id| put.iter().find(|m| m.id == id).cloned());
    for m in put {
        if let Some(x) = p.masters.iter_mut().find(|x| x.id == m.id)
            && !Arc::ptr_eq(x, m)
        {
            *x = m.clone();
        }
    }

    order_work(op.slides.edits.len(), p.slides.len())?;
    let added = &op.slides.added;
    seq::apply(&mut p.slides, &op.slides.edits, |s| s.id, |id| added.iter().find(|s| s.id == id).cloned());
    for change in &op.slides.changed {
        // A slide someone else removed meanwhile: its changes go with it.
        let Some(slide) = p.slides.iter_mut().find(|s| s.id == change.id) else { continue };
        let s = Arc::make_mut(slide);
        apply_slide_settings(s, &change.fields)?;
        order_work(change.shapes.edits.len(), s.shapes.len())?;
        let put = &change.shapes.put;
        seq::apply(&mut s.shapes, &change.shapes.edits, |x| x.id, |id| put.iter().find(|x| x.id == id).cloned());
        let inserted = inserts(&change.shapes.edits);
        for sh in put.iter().filter(|sh| !inserted.contains(&sh.id)) {
            if let Some(x) = s.shapes.iter_mut().find(|x| x.id == sh.id) {
                *x = sh.clone();
            }
        }
        if !change.shapes.is_empty() {
            dedupe_shapes(&mut s.shapes);
        }
    }

    if !op.media_removed.is_empty() {
        let used: HashSet<MediaId> = crate::media_refs::presentation_media(p).into_iter().collect();
        p.media.retain(|m| !op.media_removed.contains(&m.id) || used.contains(&m.id));
    }
    normalize(p);
    Ok(())
}

/// Two edits made at once can put one shape in two places (two people grouping the same shapes):
/// the first in stacking order keeps it, and a group left empty goes.
fn dedupe_shapes(shapes: &mut Vec<Shape>) {
    let mut ids = HashSet::new();
    let mut twice = false;
    walk(shapes, &mut |sh, _| twice |= !ids.insert(sh.id));
    if twice {
        keep_first(shapes, &mut HashSet::new(), 0);
    }
}

fn keep_first(shapes: &mut Vec<Shape>, seen: &mut HashSet<ShapeId>, depth: usize) {
    shapes.retain_mut(|sh| {
        if !seen.insert(sh.id) {
            return false;
        }
        match sh.children_mut() {
            Some(kids) if depth < 64 => {
                keep_first(kids, seen, depth + 1);
                !kids.is_empty()
            }
            _ => true,
        }
    });
}

fn order_work(edits: usize, items: usize) -> Result<()> {
    if edits.saturating_mul(items.saturating_add(edits)) > MAX_ORDER_WORK {
        return Err(refused("the edit reorders too much to share"));
    }
    Ok(())
}

/// Puts right what two edits made at once can leave behind (each fine on its own): a slide whose
/// layout went with a master someone else removed, sections and custom shows naming slides that
/// are gone, a slide someone added in a section someone else removed. A presentation a command
/// left is already like this, so this changes nothing there.
fn normalize(p: &mut Presentation) {
    let layouts: HashSet<LayoutId> = p.masters.iter().flat_map(|m| m.layouts.iter().map(|l| l.id)).collect();
    if let Some(first) = p.masters.first().and_then(|m| m.layouts.first()).map(|l| l.id) {
        for s in p.slides.iter_mut().filter(|s| !layouts.contains(&s.layout)) {
            Arc::make_mut(s).layout = first;
        }
    }
    let exists: HashSet<SlideId> = p.slides.iter().map(|s| s.id).collect();
    for show in &mut p.custom_shows {
        if show.slides.iter().any(|id| !exists.contains(id)) {
            show.slides.retain(|id| exists.contains(id));
        }
    }
    crate::cmd::slide::tidy_sections(p);
}

/// `a` rebased to apply after `b` (both made against the same presentation). `a_first`: `a` is
/// the earlier in the log, so where both change the same thing (a setting, a shape, a master,
/// a slide's place) `b`'s change stays. A change to something `b` removed goes.
pub fn rebase(a: &Op, b: &Op, a_first: bool) -> Op {
    let settings = |mine: &Fields, theirs: &Fields| -> Fields {
        mine.iter().filter(|(k, _)| !(a_first && theirs.contains_key(*k))).map(|(k, v)| (k.clone(), v.clone())).collect()
    };

    let mut out = Op { fields: settings(&a.fields, &b.fields), media_removed: a.media_removed.clone(), ..Op::default() };
    let b_media: HashSet<MediaId> = b.media.iter().map(|m| m.id).collect();
    out.media = a.media.iter().filter(|m| !(a_first && b_media.contains(&m.id))).cloned().collect();

    let gone: HashSet<MasterId> = removals(&b.masters.edits);
    let theirs: HashSet<MasterId> = b.masters.put.iter().map(|m| m.id).collect();
    out.masters.edits = seq::rebase(&a.masters.edits, &b.masters.edits, a_first);
    let kept: HashSet<MasterId> = inserts(&out.masters.edits);
    let dropped: HashSet<MasterId> = inserts(&a.masters.edits).difference(&kept).copied().collect();
    out.masters.put = a
        .masters
        .put
        .iter()
        .filter(|m| !dropped.contains(&m.id) && (kept.contains(&m.id) || !(gone.contains(&m.id) || (a_first && theirs.contains(&m.id)))))
        .cloned()
        .collect();

    let gone: HashSet<SlideId> = removals(&b.slides.edits);
    out.slides.edits = seq::rebase(&a.slides.edits, &b.slides.edits, a_first);
    let kept: HashSet<SlideId> = inserts(&out.slides.edits);
    out.slides.added = a.slides.added.iter().filter(|s| kept.contains(&s.id)).cloned().collect();
    let changed: HashMap<SlideId, &SlideChange> = b.slides.changed.iter().map(|c| (c.id, c)).collect();
    for c in &a.slides.changed {
        if gone.contains(&c.id) {
            continue;
        }
        let c = match changed.get(&c.id) {
            Some(theirs) => {
                SlideChange { id: c.id, fields: settings(&c.fields, &theirs.fields), shapes: rebase_shapes(&c.shapes, &theirs.shapes, a_first) }
            }
            None => c.clone(),
        };
        if !c.is_empty() {
            out.slides.changed.push(c);
        }
    }
    out
}

fn rebase_shapes(a: &ShapesPatch, b: &ShapesPatch, a_first: bool) -> ShapesPatch {
    let gone: HashSet<ShapeId> = removals(&b.edits);
    let theirs: HashSet<ShapeId> = b.put.iter().map(|s| s.id).collect();
    let edits = seq::rebase(&a.edits, &b.edits, a_first);
    let kept = inserts(&edits);
    let dropped: HashSet<ShapeId> = inserts(&a.edits).difference(&kept).copied().collect();
    let put = a
        .put
        .iter()
        .filter(|s| !dropped.contains(&s.id) && (kept.contains(&s.id) || !(gone.contains(&s.id) || (a_first && theirs.contains(&s.id)))))
        .cloned()
        .collect();
    ShapesPatch { edits, put }
}

fn removals<I: Copy + Eq + std::hash::Hash>(edits: &[SeqEdit<I>]) -> HashSet<I> {
    edits.iter().filter(|e| matches!(e, SeqEdit::Remove { .. })).map(|e| e.id()).collect()
}

fn inserts<I: Copy + Eq + std::hash::Hash>(edits: &[SeqEdit<I>]) -> HashSet<I> {
    edits.iter().filter(|e| matches!(e, SeqEdit::Insert { .. })).map(|e| e.id()).collect()
}

// --- Ids --------------------------------------------------------------------------------------

/// Where client ids' ranges start: above the ids files and new presentations use.
const ID_BASE: u32 = 0x4000_0000;
/// Ids in each client's range.
const ID_SPAN: u32 = 1 << 20;
/// Ranges: client ids share one when they're this far apart.
const ID_SLOTS: u64 = 1024;

/// The ids new slides, shapes, layouts, masters and media come from for client `client`, as
/// `lo..hi`: a range of its own, so two people adding a slide at once never pick the same id.
pub fn id_space(client: u64) -> (u32, u32) {
    let slot = u32::try_from(client % ID_SLOTS).unwrap_or(0);
    let lo = ID_BASE + slot * ID_SPAN;
    (lo, lo + ID_SPAN)
}

/// Points `p`'s id counter into `lo..hi`, past any id already taken there (an earlier session's
/// client with the same range made them).
pub fn claim_ids(p: &mut Presentation, (lo, hi): (u32, u32)) {
    let mut top = lo;
    let mut see = |id: u32| {
        if (lo..hi).contains(&id) {
            top = top.max(id);
        }
    };
    let masters = p.masters.iter().map(|m| &**m).chain(p.notes_master.as_deref()).chain(p.handout_master.as_deref());
    for m in masters {
        see(m.id.0);
        walk(&m.shapes, &mut |sh, _| see(sh.id.0));
        for l in &m.layouts {
            see(l.id.0);
            walk(&l.shapes, &mut |sh, _| see(sh.id.0));
        }
    }
    for s in &p.slides {
        see(s.id.0);
        walk(&s.shapes, &mut |sh, _| see(sh.id.0));
    }
    for m in &p.media {
        see(m.id.0);
    }
    p.next_id = top;
}

#[cfg(test)]
mod tests;
