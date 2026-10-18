//! One person's copy of a shared presentation.
//!
//! A server (the host's business; nothing here does I/O) orders everyone's ops into one log. Each
//! copy keeps the presentation as of its last log entry (`confirmed`) and at most one op of its
//! own the server hasn't echoed yet (`inflight`). What the person sees (`shown`, the session's
//! presentation) is the truth for them: the next op sent is always the difference between what
//! the server will have once the op in flight is in (`assumed`) and what's shown. Whatever happens
//! on the way (an op that lands differently than this copy assumed, one the server refused), the
//! next op puts it right, and nothing made is dropped.
//!
//! Convergence: log op `n` was made against log position `base`, so it's rebased over the log
//! entries `base+1 .. n-1` (its author hadn't seen them) before it's applied. That's a pure
//! function of the log, so every copy, the author's included, applies the same thing and ends
//! with the same presentation. An op the apply refuses is a no-op everywhere.
//!
//! New slides and shapes get ids from this person's own range ([`super::id_space`]): the
//! presentation's id counter (`next_id`) is each copy's own and never travels.
//!
//! Undo is each person's own: an edit's inverse goes on their undo stack, is rebased over what
//! the others do afterwards (where someone changed the same thing later, their change stays),
//! and undoing sends it to everyone like any other edit.

use std::collections::VecDeque;
use std::sync::Arc;

use deckcraft_model::Presentation;
use serde::{Deserialize, Serialize};

use super::{Op, apply, capture, claim_ids, id_space, rebase, refused};
use crate::Result;

/// Log entries kept for rebasing late ops. An op made against an older position can't be
/// placed; its author has to rejoin from a fresh copy.
const LOG_KEEP: usize = 4096;
/// Undo steps kept.
const UNDO_KEEP: usize = 100;
/// Edits joined into one undo step at most; the next starts a new step.
const STEP_OPS: usize = 500;

/// An op on its way to the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outgoing {
    pub client: u64,
    /// This client's count of its ops, so the echo can be matched.
    pub client_seq: u64,
    /// The last log position the op's author had applied.
    pub base: u64,
    pub op: Op,
}

/// A log entry from the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Incoming {
    pub seq: u64,
    pub client: u64,
    pub client_seq: u64,
    pub base: u64,
    pub op: Op,
}

#[derive(Clone, Debug)]
struct Inflight {
    client_seq: u64,
    /// Rebased over what arrived since it was sent: how it will land.
    current: Op,
}

/// An undo or redo step: the ops that take an edit back, applied in order (a run of typing is
/// one step of several), and the edit's name.
#[derive(Clone, Debug)]
struct Step {
    ops: Vec<Op>,
    label: String,
}

/// What [`Replica::receive`] did to what's shown.
#[derive(Debug)]
pub enum Shown {
    /// Nothing (this person's own op came back).
    Same,
    /// This op was applied to it.
    Applied(Op),
    /// It was rebuilt; this op turns the old into the new.
    Rebuilt(Op),
}

pub struct Replica {
    client: u64,
    /// This person's id range, `lo..hi`.
    ids: (u32, u32),
    confirmed: Arc<Presentation>,
    seq: u64,
    log: VecDeque<(u64, Op)>,
    inflight: Option<Inflight>,
    /// `confirmed` with the op in flight: what this person's edits since are made on. It shares
    /// slides with what's shown where they're the same, so diffing them is cheap.
    assumed: Arc<Presentation>,
    next_client_seq: u64,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Why the last edit that couldn't be shared was taken back, for telling the person.
    refusal: Option<String>,
}

impl Replica {
    /// Joins at log position `seq` with `shown`, the presentation as of that position. `client`
    /// comes from the server: nonzero and unique in the session. From now on new ids in `shown`
    /// come from this person's range.
    pub fn new(client: u64, shown: &mut Arc<Presentation>, seq: u64) -> Result<Replica> {
        if client == 0 {
            return Err(refused("the client id must not be 0"));
        }
        let ids = id_space(client);
        claim_ids(Arc::make_mut(shown), ids);
        Ok(Replica {
            client,
            ids,
            confirmed: shown.clone(),
            assumed: shown.clone(),
            seq,
            log: VecDeque::new(),
            inflight: None,
            next_client_seq: 1,
            undo: Vec::new(),
            redo: Vec::new(),
            refusal: None,
        })
    }

    pub fn client(&self) -> u64 {
        self.client
    }

    /// The last log position applied.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// The presentation as of [`Replica::seq`], without this person's unconfirmed edits: what a
    /// checkpoint saves.
    pub fn confirmed(&self) -> &Arc<Presentation> {
        &self.confirmed
    }

    /// Edits of this person's (in `shown`) the server hasn't confirmed yet.
    pub fn has_pending(&self, shown: &Presentation) -> bool {
        self.inflight.is_some() || capture(&self.assumed, shown).is_ok_and(|op| !op.is_empty())
    }

    pub fn can_undo(&self, redo: bool) -> bool {
        if redo { !self.redo.is_empty() } else { !self.undo.is_empty() }
    }

    /// The undo (or redo) steps' names, newest first.
    pub fn labels(&self, redo: bool) -> Vec<&str> {
        let s = if redo { &self.redo } else { &self.undo };
        s.iter().rev().map(|s| s.label.as_str()).collect()
    }

    /// This person's edit, which turned `before` into `after` (what they see now): its inverse
    /// goes on the undo stack. The edit itself goes to the others with the next
    /// [`Replica::outgoing`]. `join`: it carries on the last undo step (a run of typing). False
    /// when it changed nothing the others see (no undo step). An error means the edit can't be
    /// shared: take it back.
    pub fn local(&mut self, before: &Presentation, after: &Presentation, label: &str, join: bool) -> Result<bool> {
        let (lo, hi) = self.ids;
        if !(lo..hi).contains(&after.next_id) {
            return Err(refused("this edit can't be shared: there are no more ids for new slides and shapes in this session"));
        }
        let inverse = capture(after, before)?;
        if inverse.is_empty() {
            return Ok(false);
        }
        self.redo.clear();
        if join
            && let Some(top) = self.undo.last_mut()
            && top.ops.len() < STEP_OPS
        {
            // Taken back first, then what the step had.
            top.ops.insert(0, inverse);
            return Ok(true);
        }
        self.undo.push(Step { ops: vec![inverse], label: label.to_string() });
        if self.undo.len() > UNDO_KEEP {
            self.undo.remove(0);
        }
        Ok(true)
    }

    /// Takes back this person's last edit (or with `redo`, puts back the last one taken back) on
    /// `shown`; the others' edits since stay. `None`: nothing to take back.
    pub fn undo(&mut self, shown: &mut Arc<Presentation>, redo: bool) -> Result<Option<Op>> {
        let stack = if redo { &mut self.redo } else { &mut self.undo };
        let Some(step) = stack.pop() else { return Ok(None) };
        let mut next = (**shown).clone();
        for op in &step.ops {
            apply(&mut next, op)?;
        }
        let back = capture(&next, shown)?;
        let applied = capture(shown, &next)?;
        *shown = Arc::new(next);
        let back = Step { ops: vec![back], label: step.label };
        if redo {
            self.undo.push(back);
        } else {
            self.redo.push(back);
        }
        Ok(Some(applied))
    }

    /// The next op to send: what's shown that the server doesn't have yet. `None` while an op is
    /// on its way (the next goes once it's back) or when there's nothing new.
    ///
    /// A difference that would change nothing where it lands (a picture removed that someone
    /// else's slide uses again) or that every copy would refuse isn't sent: what's shown takes
    /// the shared presentation's version of it instead.
    pub fn outgoing(&mut self, shown: &mut Arc<Presentation>) -> Option<Outgoing> {
        if self.inflight.is_some() {
            return None;
        }
        let op = capture(&self.assumed, shown).ok().filter(|op| !op.is_empty())?;
        let mut landed = (*self.assumed).clone();
        let effective = match apply(&mut landed, &op) {
            Ok(()) => capture(&self.assumed, &landed).is_ok_and(|c| !c.is_empty()),
            Err(e) => {
                self.refusal = Some(e.to_string());
                false
            }
        };
        if !effective {
            self.resync(shown);
            return None;
        }
        let client_seq = self.next_client_seq;
        self.next_client_seq += 1;
        self.inflight = Some(Inflight { client_seq, current: op.clone() });
        self.assumed = shown.clone();
        Some(Outgoing { client: self.client, client_seq, base: self.seq, op })
    }

    /// Why this person's last unshareable edit was taken back (too large to share), once.
    pub fn take_refusal(&mut self) -> Option<String> {
        self.refusal.take()
    }

    /// The server refused the op in flight for good (too large, no permission): this person's
    /// unconfirmed edits go, and what's shown is the shared presentation again.
    pub fn drop_inflight(&mut self, shown: &mut Arc<Presentation>) -> Option<Op> {
        self.inflight.take()?;
        self.assumed = self.confirmed.clone();
        let fix = capture(shown, &self.confirmed).unwrap_or_default();
        self.resync(shown);
        (!fix.is_empty()).then_some(fix)
    }

    /// What's shown becomes `assumed`, keeping this person's id counter.
    fn resync(&self, shown: &mut Arc<Presentation>) {
        let next_id = shown.next_id;
        let mut p = (*self.assumed).clone();
        p.next_id = next_id;
        *shown = Arc::new(p);
    }

    /// Applies the server's next log entry and shows the result in `shown` (with this person's
    /// unconfirmed edits on top). An error means this copy can't follow the log (an entry missing
    /// or out of order): rejoin from a fresh copy.
    pub fn receive(&mut self, msg: Incoming, shown: &mut Arc<Presentation>) -> Result<Shown> {
        if msg.seq != self.seq + 1 {
            return Err(refused(format!("expected log entry {}, got {}", self.seq + 1, msg.seq)));
        }
        let own = msg.client == self.client;
        if own && self.inflight.as_ref().is_none_or(|p| p.client_seq != msg.client_seq) {
            return Err(refused("an echo of an op this copy didn't send"));
        }
        // Without the entries its author hadn't seen, this copy can't place the op the way the
        // others do: rejoin from a fresh copy rather than diverge.
        let oldest = self.log.front().map_or(self.seq + 1, |(s, _)| *s);
        if msg.base < self.seq && msg.base + 1 < oldest {
            return Err(refused("the op is older than this copy's log; rejoin the session"));
        }
        let (op, doc) = match self.place(&msg) {
            Ok(placed) => placed,
            // Refused the same way everywhere: a no-op in the log.
            Err(_) => (Op::default(), self.confirmed.clone()),
        };
        self.confirmed = doc;
        self.seq = msg.seq;
        self.log.push_back((msg.seq, op.clone()));
        while self.log.len() > LOG_KEEP {
            self.log.pop_front();
        }
        if own {
            // In the log now. If it landed differently than assumed (or was refused), what's shown
            // stays as the person made it, and the next op sends the difference.
            self.inflight = None;
            self.assumed = self.confirmed.clone();
            self.share_if_same(shown);
            return Ok(Shown::Same);
        }
        // Someone else's: it goes after the op in flight and this person's edits since.
        let unsent = capture(&self.assumed, shown).unwrap_or_default();
        let mut theirs = op;
        if let Some(p) = self.inflight.as_mut() {
            let mine = p.current.clone();
            p.current = rebase(&mine, &theirs, false);
            theirs = rebase(&theirs, &mine, true);
        }
        let mut assumed = (*self.assumed).clone();
        self.assumed = match apply(&mut assumed, &theirs) {
            Ok(()) => Arc::new(assumed),
            Err(_) => self.rebuild_assumed(),
        };
        let theirs = rebase(&theirs, &unsent, true);
        for s in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            for op in s.ops.iter_mut() {
                *op = rebase(op, &theirs, true);
            }
        }
        let mut next = (**shown).clone();
        let result = match apply(&mut next, &theirs) {
            Ok(()) => {
                *shown = Arc::new(next);
                Shown::Applied(theirs)
            }
            Err(_) => {
                // Shouldn't happen; show what the server has with this person's edits on top as
                // far as they still apply.
                let mut p = (*self.assumed).clone();
                if apply(&mut p, &rebase(&unsent, &theirs, false)).is_err() {
                    p = (*self.assumed).clone();
                }
                p.next_id = shown.next_id;
                let fix = capture(shown, &p).unwrap_or_default();
                *shown = Arc::new(p);
                Shown::Rebuilt(fix)
            }
        };
        self.share_if_same(shown);
        Ok(result)
    }

    /// `confirmed` with the op in flight, rebuilt.
    fn rebuild_assumed(&self) -> Arc<Presentation> {
        if let Some(p) = &self.inflight {
            let mut next = (*self.confirmed).clone();
            if apply(&mut next, &p.current).is_ok() {
                return Arc::new(next);
            }
        }
        self.confirmed.clone()
    }

    /// With nothing of this person's unconfirmed, the confirmed presentation takes what's shown
    /// (the same content), so they share their slides again and the next diff is cheap.
    fn share_if_same(&mut self, shown: &Arc<Presentation>) {
        if self.inflight.is_none() && capture(&self.assumed, shown).is_ok_and(|op| op.is_empty()) {
            self.confirmed = shown.clone();
            self.assumed = shown.clone();
        }
    }

    /// Log entry `msg` rebased over the entries its author hadn't seen, and the confirmed
    /// presentation with it.
    fn place(&self, msg: &Incoming) -> Result<(Op, Arc<Presentation>)> {
        if msg.base >= msg.seq {
            return Err(refused("an op made against a later position"));
        }
        let mut op = msg.op.clone();
        for (_, m) in self.log.iter().filter(|(s, _)| *s > msg.base && *s < msg.seq) {
            op = rebase(&op, m, false);
        }
        let mut doc = (*self.confirmed).clone();
        apply(&mut doc, &op)?;
        Ok((op, Arc::new(doc)))
    }
}
