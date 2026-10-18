//! Co-editing through a [`Session`]: once started, every edit of the shared presentation (from
//! any command path: ribbon, keys, pointer gestures, agents) becomes an op for the others, Undo
//! and Redo take back this person's own edits, and the host feeds the server's log back in.
//!
//! An edit is noticed by comparing the presentation with the one last shared, after every
//! outermost command and whenever the host asks for the next op ([`Session::collab_outgoing`]),
//! so nothing has to call in. An edit starts a new undo step when the session's own history got a
//! new step for it; otherwise (a run of typing) it carries on the last one.

use std::sync::Arc;

use deckcraft_model::{Presentation, ShapeId, SlideId};
use serde_json::{Value, json};

use super::{Incoming, Outgoing, Replica, Shown, apply};
use crate::{DocState, EngineError, Result, Session, Target};

/// The most people whose places are kept.
const MAX_OTHERS: usize = 64;
/// The most shapes of one person's selection kept.
const MAX_OTHER_SHAPES: usize = 256;
/// The longest name kept, in characters.
const MAX_NAME_CHARS: usize = 60;
/// The largest state snapshot read (base64 text).
pub const MAX_SNAPSHOT_BYTES: usize = 700 << 20;

pub(crate) const READ_ONLY: &str = "You can view this presentation but not edit it.";

/// The shared presentation of a [`Session`] and this person's copy of it.
pub(crate) struct Shared {
    pub(crate) replica: Replica,
    /// The shared presentation's document ([`DocState::uid`]): closing it ends sharing, and other
    /// open presentations aren't shared.
    pub(crate) uid: u64,
    /// The presentation as last shared: the next edit is what changed since.
    shown: Arc<Presentation>,
    /// The session's undo step on top then (its snapshot's address): a new one starts a new undo
    /// step here too.
    top: Option<usize>,
    read_only: bool,
    /// Where the others are, as they last reported.
    others: Vec<Other>,
    /// Something to tell this person (an edit that couldn't be shared), once.
    notice: Option<String>,
}

/// Someone else in the session, for drawing: the slide they're on and the shapes they selected.
#[derive(Clone, Debug, PartialEq)]
pub struct Other {
    pub client: u64,
    pub name: String,
    pub color: [u8; 3],
    pub slide: Option<SlideId>,
    pub shapes: Vec<ShapeId>,
}

impl Other {
    /// `name` as shown: one line, at most 60 characters; at most 256 shapes.
    pub fn new(client: u64, name: &str, color: [u8; 3], slide: Option<SlideId>, mut shapes: Vec<ShapeId>) -> Other {
        shapes.truncate(MAX_OTHER_SHAPES);
        let name = name.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect();
        Other { client, name, color, slide, shapes }
    }
}

fn top_of(st: &DocState) -> Option<usize> {
    st.history.undo.last().map(|e| Arc::as_ptr(&e.doc) as usize)
}

/// Mid-gesture or mid-composition, the presentation isn't an edit yet.
fn settled(st: &DocState) -> bool {
    st.interaction.is_none() && st.composition.is_none()
}

impl Session {
    /// Shares the active presentation, which is the shared one as of log position `seq`; `client`
    /// is this person's id from the server.
    pub fn collab_start(&mut self, client: u64, seq: u64) -> Result<()> {
        let st = self.doc_mut()?;
        let replica = Replica::new(client, &mut st.doc, seq)?;
        let (uid, shown, top) = (st.uid, st.doc.clone(), top_of(st));
        st.revision += 1;
        self.collab = Some(Box::new(Shared { replica, uid, shown, top, read_only: false, others: Vec::new(), notice: None }));
        Ok(())
    }

    /// Replaces the active presentation with a checkpoint's state snapshot
    /// ([`Session::collab_snapshot`]): the exact presentation the others have, which re-reading
    /// the saved .pptx wouldn't give (it numbers slides afresh). Call before
    /// [`Session::collab_start`]; the presentation counts as saved.
    pub fn collab_load_snapshot(&mut self, state: &str) -> Result<()> {
        if state.len() > MAX_SNAPSHOT_BYTES {
            return Err(EngineError::Other("co-editing: the shared presentation is too large".into()));
        }
        let bytes =
            crate::cmd::base64_decode(state.trim()).ok_or_else(|| EngineError::Other("co-editing: the shared presentation can't be read".into()))?;
        let p = deckcraft_format::load(&bytes).map_err(|e| EngineError::Other(format!("co-editing: the shared presentation can't be read: {e}")))?;
        let st = self.doc_mut()?;
        st.doc = Arc::new(p);
        st.saved_doc = st.doc.clone();
        st.history.undo.clear();
        st.history.redo.clear();
        st.interaction = None;
        st.composition = None;
        st.selection = Default::default();
        st.revision += 1;
        Ok(())
    }

    /// The shared presentation as everyone has it (without this person's unconfirmed edits) and
    /// its log position, as a state snapshot (base64 of the native format): what a checkpoint
    /// keeps beside the saved file for people who join from it.
    pub fn collab_snapshot(&self) -> Option<Result<(u64, String)>> {
        let c = self.collab.as_ref()?;
        Some(
            deckcraft_format::save(c.replica.confirmed())
                .map(|bytes| (c.replica.seq(), crate::cmd::base64_encode(&bytes)))
                .map_err(|e| EngineError::Other(format!("co-editing: {e}"))),
        )
    }

    /// The shared presentation as everyone has it and its log position: what a checkpoint saves.
    pub fn collab_confirmed(&self) -> Option<(u64, &Arc<Presentation>)> {
        self.collab.as_ref().map(|c| (c.replica.seq(), c.replica.confirmed()))
    }

    /// This person can view the shared presentation but not edit it: an edit is refused (or taken
    /// back) with a message; moving between slides, selecting and copying still work.
    pub fn collab_set_read_only(&mut self, read_only: bool) {
        if let Some(c) = self.collab.as_mut() {
            c.read_only = read_only;
        }
    }

    pub fn collab_read_only(&self) -> bool {
        self.collab.as_ref().is_some_and(|c| c.read_only)
    }

    pub fn collab_active(&self) -> bool {
        self.collab.is_some()
    }

    pub fn collab_stop(&mut self) {
        self.collab = None;
    }

    /// This person's client id while the presentation is shared.
    pub fn collab_client(&self) -> Option<u64> {
        self.collab.as_ref().map(|c| c.replica.client())
    }

    /// The last log position applied.
    pub fn collab_seq(&self) -> Option<u64> {
        self.collab.as_ref().map(|c| c.replica.seq())
    }

    /// This person has edits the server hasn't confirmed.
    pub fn collab_pending(&self) -> bool {
        let Some(c) = self.collab.as_ref() else { return false };
        self.docs.iter().find(|d| d.uid == c.uid).is_some_and(|d| c.replica.has_pending(&d.doc))
    }

    /// Whether `uid` is the shared presentation.
    pub fn collab_shares(&self, uid: u64) -> bool {
        self.collab.as_ref().is_some_and(|c| c.uid == uid)
    }

    /// Something to tell this person about co-editing (an edit that couldn't be shared), once.
    pub fn collab_take_notice(&mut self) -> Option<String> {
        let c = self.collab.as_mut()?;
        c.notice.take().or_else(|| c.replica.take_refusal().map(|e| format!("An edit couldn't be shared and was taken back: {e}")))
    }

    /// The next op to send to the server (one at a time: the next once this one comes back).
    /// Nothing while a gesture or an input method's composition is in progress: it goes when done.
    pub fn collab_outgoing(&mut self) -> Option<Outgoing> {
        self.collab_flush();
        let c = self.collab.as_mut()?;
        let st = self.docs.iter_mut().find(|d| d.uid == c.uid)?;
        if !settled(st) {
            return None;
        }
        let before = st.doc.clone();
        let out = c.replica.outgoing(&mut st.doc);
        if !Arc::ptr_eq(&before, &st.doc) {
            // Took the shared version of something that couldn't be sent.
            st.revision += 1;
            crate::clamp_selection(st);
        }
        c.shown = st.doc.clone();
        out
    }

    /// The server refused the op in flight for good: drop it here too, and show the shared
    /// presentation again.
    pub fn collab_refused(&mut self) {
        let Some(c) = self.collab.as_mut() else { return };
        let Some(st) = self.docs.iter_mut().find(|d| d.uid == c.uid) else { return };
        if c.replica.drop_inflight(&mut st.doc).is_some() {
            st.revision += 1;
            st.interaction = None;
            crate::clamp_selection(st);
        }
        c.shown = st.doc.clone();
    }

    /// Applies the server's next log entry. An error means this copy can't follow the log (an
    /// entry missing or out of order): rejoin from a fresh copy.
    pub fn collab_receive(&mut self, msg: Incoming) -> Result<()> {
        self.collab_flush();
        let Some(c) = self.collab.as_mut() else { return Err(EngineError::Other("co-editing: no shared presentation".into())) };
        let Some(st) = self.docs.iter_mut().find(|d| d.uid == c.uid) else {
            return Err(EngineError::Other("co-editing: the shared presentation was closed".into()));
        };
        let shown = c.replica.receive(msg, &mut st.doc)?;
        match &shown {
            Shown::Same => {}
            Shown::Applied(op) => {
                // A gesture or a composition in progress started from the presentation before the
                // op: it goes on from the presentation with it.
                if let Some(i) = st.interaction.as_mut() {
                    let mut base = (*i.doc).clone();
                    match apply(&mut base, op) {
                        Ok(()) => i.doc = Arc::new(base),
                        Err(_) => st.interaction = None,
                    }
                }
                if let Some(comp) = st.composition.as_mut() {
                    let mut base = (*comp.before).clone();
                    if apply(&mut base, op).is_ok() {
                        comp.before = Arc::new(base);
                    }
                }
                st.revision += 1;
            }
            Shown::Rebuilt(_) => {
                st.interaction = None;
                st.composition = None;
                st.revision += 1;
            }
        }
        crate::clamp_selection(st);
        // Mid-gesture, what's shared is where the gesture started (with the op): the gesture
        // goes out whole, as one edit, when it ends.
        c.shown = match (&st.interaction, &st.composition) {
            (Some(i), _) => i.doc.clone(),
            (None, Some(comp)) => comp.before.clone(),
            (None, None) => st.doc.clone(),
        };
        Ok(())
    }

    /// Where this person is, for the others to draw: `{slide, shapes}` (the slide shown, the
    /// shapes selected on it; `slide` null in the master views).
    pub fn collab_presence(&self) -> Value {
        let Some(st) = self.collab.as_ref().and_then(|c| self.docs.iter().find(|d| d.uid == c.uid)) else { return Value::Null };
        let slide = (st.selection.target == Target::Slides).then(|| st.current_slide().map(|s| s.id.0)).flatten();
        let shapes: Vec<u32> = if slide.is_some() { st.selection.shapes.iter().take(MAX_OTHER_SHAPES).map(|s| s.0).collect() } else { Vec::new() };
        json!({"slide": slide, "shapes": shapes})
    }

    /// Where the others are, as they last reported ([`Session::collab_presence`] of theirs).
    pub fn collab_set_others(&mut self, mut others: Vec<Other>) {
        if let Some(c) = self.collab.as_mut() {
            others.truncate(MAX_OTHERS);
            c.others = others;
        }
    }

    /// The others in the session, for drawing.
    pub fn collab_others(&self) -> &[Other] {
        self.collab.as_ref().map_or(&[], |c| c.others.as_slice())
    }

    /// Records what changed in the shared presentation since it was last shared as this person's
    /// edit: queued for the others, its inverse on their undo stack. A view-only person's edit,
    /// or one that can't be shared, is taken back with a notice.
    pub(crate) fn collab_flush(&mut self) {
        let Some(c) = self.collab.as_mut() else { return };
        let Some(st) = self.docs.iter_mut().find(|d| d.uid == c.uid) else {
            // The shared presentation was closed.
            self.collab = None;
            return;
        };
        if !settled(st) || Arc::ptr_eq(&st.doc, &c.shown) {
            return;
        }
        let top = top_of(st);
        let outcome = if c.read_only {
            Err(READ_ONLY.to_string())
        } else {
            let label = st.history.undo.last().map(|e| e.label.clone()).unwrap_or_default();
            c.replica.local(&c.shown, &st.doc, &label, top == c.top).map_err(|e| e.to_string())
        };
        match outcome {
            Ok(_) => c.shown = st.doc.clone(),
            Err(e) => {
                st.doc = c.shown.clone();
                st.revision += 1;
                crate::clamp_selection(st);
                c.notice = Some(e);
            }
        }
        c.top = top;
    }

    /// Whether Undo (or Redo) has something of this person's to take back; `None` when the active
    /// presentation isn't shared (its own history decides).
    pub(crate) fn collab_can_undo(&self, redo: bool) -> Option<bool> {
        let c = self.collab.as_ref()?;
        (self.active()?.uid == c.uid).then(|| c.replica.can_undo(redo) && !c.read_only)
    }

    /// Undo (or Redo) in the shared presentation: this person's own last edit; `None` when the
    /// active presentation isn't shared.
    pub(crate) fn collab_undo(&mut self, redo: bool) -> Option<Result<Value>> {
        let uid = self.active()?.uid;
        if !self.collab_shares(uid) {
            return None;
        }
        self.collab_flush();
        let c = self.collab.as_mut()?;
        let st = self.docs.iter_mut().find(|d| d.uid == c.uid)?;
        if c.read_only {
            return Some(Err(EngineError::Other(READ_ONLY.into())));
        }
        let label = if redo { c.replica.labels(true) } else { c.replica.labels(false) }.first().map(|l| l.to_string()).unwrap_or_default();
        Some(match c.replica.undo(&mut st.doc, redo) {
            Ok(Some(_)) => {
                st.interaction = None;
                st.revision += 1;
                crate::clamp_selection(st);
                c.shown = st.doc.clone();
                Ok(json!({"label": label}))
            }
            Ok(None) => Ok(json!({})),
            Err(e) => Err(e),
        })
    }
}
