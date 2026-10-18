//! Co-editing: scenarios two people run into, and the convergence fuzz (two or three people run
//! random commands, exchange ops through a fake server in random order, and must end with the
//! same presentation).

use std::collections::VecDeque;
use std::sync::Arc;

use deckcraft_model::{Presentation, Shape, ShapeId, SlideId};
use serde_json::{Value, json};

use super::*;
use crate::{DocState, EngineError, Session, command_specs, sample};

// --- A fake server and the people around it -------------------------------------------------

/// The sequencer: numbers the ops it gets and hands out the log. Ops travel as JSON, as they
/// will between browsers.
#[derive(Default)]
struct Server {
    log: Vec<String>,
}

struct Person {
    s: Session,
    r: Replica,
    /// Log entries applied.
    delivered: usize,
}

impl Person {
    fn doc(&self) -> &Arc<Presentation> {
        &self.s.doc().unwrap().doc
    }

    fn shown(&mut self) -> &mut Arc<Presentation> {
        &mut self.s.doc_mut().unwrap().doc
    }

    /// Sends this person's next op to the server, if they have one ready.
    fn push(&mut self, server: &mut Server) -> bool {
        let Some(out) = self.r.outgoing(&mut self.s.doc_mut().unwrap().doc) else { return false };
        let out: Outgoing = serde_json::from_str(&serde_json::to_string(&out).unwrap()).unwrap();
        let seq = server.log.len() as u64 + 1;
        let msg = Incoming { seq, client: out.client, client_seq: out.client_seq, base: out.base, op: out.op };
        server.log.push(serde_json::to_string(&msg).unwrap());
        true
    }

    /// Applies the next log entry this person hasn't had.
    fn deliver_one(&mut self, server: &Server) -> bool {
        let Some(msg) = server.log.get(self.delivered) else { return false };
        let msg: Incoming = serde_json::from_str(msg).unwrap();
        self.r.receive(msg, &mut self.s.doc_mut().unwrap().doc).unwrap();
        self.s.doc_mut().unwrap().revision += 1;
        self.s.select(|_, _| {}).unwrap();
        self.delivered += 1;
        true
    }

    /// Runs a command for this person, as the session hook will: Undo and Redo take back their
    /// own edits; any other change is recorded for sharing (or taken back when it can't be).
    /// Whether it changed the presentation.
    fn run(&mut self, id: &str, params: &Value) -> bool {
        if id == "edit.undo" || id == "edit.redo" {
            let r = self.r.undo(&mut self.s.doc_mut().unwrap().doc, id == "edit.redo");
            self.s.select(|_, _| {}).unwrap();
            return matches!(r, Ok(Some(_)));
        }
        let before = self.doc().clone();
        let res = self.s.execute(id, params);
        if let Err(EngineError::Internal(..)) = &res {
            panic!("command panicked (caught by the guard): {res:?}; {id} {params}");
        }
        let after = self.doc().clone();
        if Arc::ptr_eq(&before, &after) {
            return false;
        }
        let label = command_specs().iter().find(|c| c.id == id).map_or(id, |c| c.label);
        match self.r.local(&before, &after, label) {
            Ok(shared) => shared,
            Err(_) => {
                *self.shown() = before;
                false
            }
        }
    }
}

fn people(n: u64) -> Vec<Person> {
    let mut base = Session::new();
    sample::open_sample(&mut base).unwrap();
    let doc = (*base.doc().unwrap().doc).clone();
    (1..=n)
        .map(|client| {
            let mut s = Session::new();
            s.add_document(DocState::new(doc.clone(), None, "Shared".into()));
            let r = Replica::new(client, &mut s.doc_mut().unwrap().doc, 0).unwrap();
            Person { s, r, delivered: 0 }
        })
        .collect()
}

/// Everyone sends what they have and applies everything, until nothing is left in flight.
fn settle(server: &mut Server, all: &mut [Person]) {
    for _ in 0..100 {
        let mut busy = false;
        for p in all.iter_mut() {
            busy |= p.push(server);
        }
        for p in all.iter_mut() {
            while p.deliver_one(server) {
                busy = true;
            }
        }
        if !busy && all.iter().all(|p| !p.r.has_pending(p.doc())) {
            return;
        }
    }
    panic!("the copies never settled");
}

/// The presentation as everyone must agree on it: without this copy's own id counter, media by
/// id with their bytes.
fn canonical(p: &Presentation) -> Value {
    let mut v = serde_json::to_value(p).unwrap();
    if let Some(m) = v.as_object_mut() {
        m.remove("nextId");
        m.remove("media");
    }
    let mut media: Vec<_> =
        p.media.iter().map(|m| (m.id.0, m.name.clone(), m.content_type.clone(), m.link.clone(), m.data.as_ref().clone())).collect();
    media.sort_by_key(|m| m.0);
    json!({"p": v, "media": serde_json::to_value(media).unwrap()})
}

fn first_diff(a: &Value, b: &Value, path: &str) -> String {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, va) in x {
                match y.get(k) {
                    Some(vb) if va != vb => return first_diff(va, vb, &format!("{path}.{k}")),
                    None => return format!("{path}.{k} only on the left"),
                    _ => {}
                }
            }
            y.keys().find(|k| !x.contains_key(*k)).map_or_else(String::new, |k| format!("{path}.{k} only on the right"))
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            x.iter().zip(y).enumerate().find(|(_, (p, q))| p != q).map_or_else(String::new, |(i, (p, q))| first_diff(p, q, &format!("{path}[{i}]")))
        }
        _ => format!("{path}: {} vs {}", short(a), short(b)),
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 300 { format!("{}…", &s[..s.floor_char_boundary(300)]) } else { s }
}

fn assert_same(all: &[Person], ctx: &str) {
    let first = canonical(all[0].doc());
    for (i, p) in all.iter().enumerate().skip(1) {
        let other = canonical(p.doc());
        assert!(first == other, "copies 1 and {} differ ({ctx}): {}", i + 1, first_diff(&first, &other, ""));
    }
}

fn slide_ids(p: &Person) -> Vec<SlideId> {
    p.doc().slides.iter().map(|s| s.id).collect()
}

fn go_to(p: &mut Person, slide: usize) {
    p.s.select(|_, sel| {
        sel.slide = slide;
        sel.shapes.clear();
        sel.text = None;
    })
    .unwrap();
}

fn shapes_on(p: &Person, slide: usize) -> Vec<&Shape> {
    p.doc().slides[slide].shapes.iter().collect()
}

// --- Scenarios --------------------------------------------------------------------------------

#[test]
fn capture_then_apply_rebuilds_the_edit() {
    let mut all = people(1);
    let p = &mut all[0];
    let before = p.doc().clone();
    p.run("slide.new", &json!({"layout": "titleAndContent", "title": "New"}));
    p.run("shape.insert", &json!({"preset": "star5", "rect": [100, 100, 200, 200]}));
    p.run("slide.move", &json!({"index": 0, "to": 2}));
    let after = p.doc().clone();
    let op = capture(&before, &after).unwrap();
    let mut rebuilt = (*before).clone();
    apply(&mut rebuilt, &op).unwrap();
    assert_eq!(canonical(&rebuilt), canonical(&after));
    // And as JSON, as it travels.
    let back: Op = serde_json::from_str(&serde_json::to_string(&op).unwrap()).unwrap();
    assert_eq!(back, op);
    assert!(capture(&after, &after).unwrap().is_empty());
}

#[test]
fn two_people_adding_a_slide_at_once_both_keep_theirs() {
    let mut server = Server::default();
    let mut all = people(2);
    let n = all[0].doc().slides.len();
    go_to(&mut all[0], 0);
    go_to(&mut all[1], 0);
    assert!(all[0].run("slide.new", &json!({"title": "Ann's"})));
    assert!(all[1].run("slide.new", &json!({"title": "Bob's"})));
    let (a, b) = (all[0].doc().slides[1].id, all[1].doc().slides[1].id);
    assert_ne!(a, b, "new slides get ids from their author's range");
    settle(&mut server, &mut all);
    assert_same(&all, "two new slides");
    let ids = slide_ids(&all[0]);
    assert_eq!(ids.len(), n + 2);
    // Both went after the first slide; the later one in the log sits right after it.
    assert_eq!(&ids[1..3], &[b, a]);
}

#[test]
fn ids_come_from_each_persons_range() {
    let all = people(3);
    for p in &all {
        let (lo, hi) = id_space(p.r.client());
        assert!((lo..hi).contains(&p.doc().next_id));
    }
    assert_ne!(id_space(1), id_space(2));
    assert_eq!(id_space(1), id_space(1 + 1024), "ranges repeat only 1,024 clients apart");
    // A range a previous session used carries on past its ids.
    let mut p = Presentation::default();
    let (lo, hi) = id_space(5);
    Arc::make_mut(&mut p.slides[0]).id = SlideId(lo + 40);
    claim_ids(&mut p, (lo, hi));
    assert_eq!(p.next_id, lo + 40);
}

#[test]
fn editing_different_shapes_of_one_slide_keeps_both() {
    let mut server = Server::default();
    let mut all = people(2);
    go_to(&mut all[0], 0);
    go_to(&mut all[1], 0);
    let ids: Vec<ShapeId> = shapes_on(&all[0], 0).iter().map(|s| s.id).collect();
    assert!(ids.len() >= 2, "the sample's first slide has a title and a subtitle");
    assert!(all[0].run("shape.move", &json!({"ids": [ids[0].0], "dx": 10, "dy": 0})));
    assert!(all[1].run("shape.move", &json!({"ids": [ids[1].0], "dx": 0, "dy": 25})));
    let moved_a = all[0].doc().slides[0].shape(ids[0]).unwrap().xfrm;
    let moved_b = all[1].doc().slides[0].shape(ids[1]).unwrap().xfrm;
    settle(&mut server, &mut all);
    assert_same(&all, "two shapes moved");
    assert_eq!(all[0].doc().slides[0].shape(ids[0]).unwrap().xfrm, moved_a);
    assert_eq!(all[0].doc().slides[0].shape(ids[1]).unwrap().xfrm, moved_b);
}

#[test]
fn the_same_shape_changed_at_once_takes_the_later_change() {
    let mut server = Server::default();
    let mut all = people(2);
    go_to(&mut all[0], 0);
    go_to(&mut all[1], 0);
    let id = shapes_on(&all[0], 0)[0].id;
    all[0].run("shape.move", &json!({"ids": [id.0], "dx": 10, "dy": 0}));
    all[1].run("shape.move", &json!({"ids": [id.0], "dx": 0, "dy": 40}));
    let bobs = all[1].doc().slides[0].shape(id).unwrap().xfrm;
    // Ann's goes in first, Bob's second: Bob's stands.
    all[0].push(&mut server);
    all[1].push(&mut server);
    settle(&mut server, &mut all);
    assert_same(&all, "one shape moved twice");
    assert_eq!(all[0].doc().slides[0].shape(id).unwrap().xfrm, bobs);
}

#[test]
fn a_slide_deleted_while_someone_edits_it_stays_deleted() {
    let mut server = Server::default();
    let mut all = people(2);
    let n = all[0].doc().slides.len();
    let gone = all[0].doc().slides[1].id;
    go_to(&mut all[1], 1);
    let id = shapes_on(&all[1], 1)[0].id;
    all[0].run("slide.delete", &json!({"index": 1}));
    all[1].run("shape.move", &json!({"ids": [id.0], "dx": 5, "dy": 5}));
    settle(&mut server, &mut all);
    assert_same(&all, "delete against edit");
    assert_eq!(all[0].doc().slides.len(), n - 1);
    assert!(all[0].doc().slide(gone).is_none());
}

#[test]
fn moving_slides_at_once_keeps_both_moves() {
    let mut server = Server::default();
    let mut all = people(2);
    let ids = slide_ids(&all[0]);
    assert!(ids.len() >= 4);
    all[0].run("slide.move", &json!({"from": 0, "to": ids.len() - 1}));
    all[1].run("slide.move", &json!({"from": 2, "to": 1}));
    settle(&mut server, &mut all);
    assert_same(&all, "two slide moves");
    let now = slide_ids(&all[0]);
    assert_eq!(now.last(), Some(&ids[0]), "Ann's slide went last");
    assert_eq!(now.iter().position(|x| *x == ids[2]).unwrap() + 1, now.iter().position(|x| *x == ids[1]).unwrap(), "Bob's went before slide 2");
}

#[test]
fn grouping_the_same_shapes_at_once_makes_one_group() {
    let mut server = Server::default();
    let mut all = people(2);
    go_to(&mut all[0], 0);
    go_to(&mut all[1], 0);
    let ids: Vec<u32> = shapes_on(&all[0], 0).iter().take(2).map(|s| s.id.0).collect();
    assert!(all[0].run("arrange.group", &json!({"ids": ids})));
    assert!(all[1].run("arrange.group", &json!({"ids": ids})));
    settle(&mut server, &mut all);
    assert_same(&all, "two groupings");
    assert!(all[0].doc().validate().is_empty());
    let groups: Vec<&Shape> = shapes_on(&all[0], 0).into_iter().filter(|s| s.is_group()).collect();
    assert_eq!(groups.len(), 1, "one group keeps the shapes, the emptied one goes");
    assert_eq!(groups[0].children().iter().map(|c| c.id.0).collect::<Vec<_>>(), ids);
}

#[test]
fn undo_takes_back_only_your_own_edit() {
    let mut server = Server::default();
    let mut all = people(2);
    go_to(&mut all[0], 0);
    go_to(&mut all[1], 0);
    let ids: Vec<ShapeId> = shapes_on(&all[0], 0).iter().map(|s| s.id).collect();
    let start = all[0].doc().slides[0].shape(ids[0]).unwrap().xfrm;
    all[0].run("shape.move", &json!({"ids": [ids[0].0], "dx": 30, "dy": 0}));
    settle(&mut server, &mut all);
    all[1].run("shape.move", &json!({"ids": [ids[1].0], "dx": 0, "dy": 30}));
    let bobs = all[1].doc().slides[0].shape(ids[1]).unwrap().xfrm;
    settle(&mut server, &mut all);
    assert!(all[0].run("edit.undo", &json!({})));
    settle(&mut server, &mut all);
    assert_same(&all, "undo");
    assert_eq!(all[1].doc().slides[0].shape(ids[0]).unwrap().xfrm, start, "Ann's move is taken back");
    assert_eq!(all[1].doc().slides[0].shape(ids[1]).unwrap().xfrm, bobs, "Bob's stays");
    assert!(all[0].run("edit.redo", &json!({})));
    settle(&mut server, &mut all);
    assert_same(&all, "redo");
    assert_ne!(all[1].doc().slides[0].shape(ids[0]).unwrap().xfrm, start);
    // Nothing of Bob's to undo on Ann's side.
    assert!(!all[0].r.labels(false).is_empty());
}

#[test]
fn undo_skips_what_someone_else_changed_since() {
    let mut server = Server::default();
    let mut all = people(2);
    go_to(&mut all[0], 0);
    go_to(&mut all[1], 0);
    let id = shapes_on(&all[0], 0)[0].id;
    all[0].run("shape.move", &json!({"ids": [id.0], "dx": 30, "dy": 0}));
    settle(&mut server, &mut all);
    all[1].run("shape.move", &json!({"ids": [id.0], "dx": 0, "dy": 30}));
    let bobs = all[1].doc().slides[0].shape(id).unwrap().xfrm;
    settle(&mut server, &mut all);
    all[0].run("edit.undo", &json!({}));
    settle(&mut server, &mut all);
    assert_same(&all, "undo after someone else's change");
    assert_eq!(all[0].doc().slides[0].shape(id).unwrap().xfrm, bobs);
}

#[test]
fn a_late_op_is_placed_after_what_its_author_missed() {
    let mut server = Server::default();
    let mut all = people(3);
    for p in all.iter_mut() {
        go_to(p, 0);
    }
    all[0].run("slide.new", &json!({"title": "A"}));
    all[1].run("slide.delete", &json!({"index": 0}));
    all[2].run("slide.new", &json!({"title": "C"}));
    for p in all.iter_mut() {
        p.push(&mut server);
    }
    settle(&mut server, &mut all);
    assert_same(&all, "three at once");
    assert!(all[0].doc().validate().is_empty());
}

#[test]
fn hostile_ops_are_refused_not_crashed_on() {
    let p = Presentation::default();
    // Too many edits.
    let mut op = Op::default();
    op.slides.edits = (0..MAX_OP_ITEMS as u32 + 1).map(|i| SeqEdit::Remove { id: SlideId(i), after: None }).collect();
    assert!(apply(&mut p.clone(), &op).is_err());
    // An unknown setting, a setting of the wrong type, damaged media.
    let mut op = Op::default();
    op.fields.insert("nope".into(), json!(1));
    assert!(apply(&mut p.clone(), &op).is_err());
    let mut op = Op::default();
    op.fields.insert("slide_size".into(), json!("big"));
    assert!(apply(&mut p.clone(), &op).is_err());
    assert!(serde_json::from_str::<Op>(r#"{"media":[{"id":3,"data":"!!!"}]}"#).is_err());
    // Edits naming slides and neighbours that aren't there are skipped.
    let mut op = Op::default();
    op.slides.edits = vec![
        SeqEdit::Move { id: SlideId(999), after: None, from: None },
        SeqEdit::Insert { id: SlideId(1000), after: Some(SlideId(5)) },
        SeqEdit::Remove { id: SlideId(77), after: None },
    ];
    op.slides.changed.push(SlideChange { id: SlideId(424242), ..Default::default() });
    let mut q = p.clone();
    apply(&mut q, &op).unwrap();
    assert_eq!(canonical(&q), canonical(&p));
}

#[test]
fn an_edit_with_ids_from_outside_the_range_is_taken_back() {
    let mut all = people(1);
    let p = &mut all[0];
    let before = p.doc().clone();
    let mut after = (*before).clone();
    after.next_id = 7;
    assert!(p.r.local(&before, &after, "x").is_err());
}

// --- The fuzz ---------------------------------------------------------------------------------

/// Commands that touch the file system, other windows or real time, or open other documents.
fn skipped(id: &str) -> bool {
    matches!(
        id,
        "file.close" | "file.open" | "file.save" | "file.saveAs" | "file.export" | "file.saveTemplate" | "file.openBytes" | "file.new" | "file.adopt"
    ) || id.starts_with("window.")
        || id.starts_with("show.")
        || id.starts_with("media.")
        || id.starts_with("review.compare")
}

/// xorshift64*: tiny, deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
    fn f(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (self.next() >> 11) as f64 / (1u64 << 53) as f64 * (hi - lo)
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> Option<&'a T> {
        v.get(self.below(v.len()))
    }
}

/// Plausible parameters built from the person's copy (the soak's, without the hostile values
/// that only make commands refuse).
fn params(r: &mut Rng, s: &Session) -> Value {
    let ids: Vec<u64> = s.active().map(|d| d.shapes().iter().map(|x| u64::from(x.id.0)).collect()).unwrap_or_default();
    let id = r.pick(&ids).copied().unwrap_or(1);
    let slides = s.active().map(|d| d.doc.slides.len()).unwrap_or(1);
    let index = r.below(slides + 1);
    match r.below(9) {
        0 => json!({}),
        1 => json!({"ids": [id]}),
        2 => json!({"id": id, "text": "Fuzz ✓ שלום 漢字", "name": "F"}),
        3 => json!({"index": index, "to": r.below(slides + 1)}),
        4 => json!({"preset": "rect", "rect": [r.f(0.0, 600.0), r.f(0.0, 400.0), r.f(1.0, 300.0), r.f(1.0, 200.0)]}),
        5 => {
            json!({"ids": [id], "color": "#3366CC", "size": r.f(1.0, 96.0), "value": r.f(-10.0, 10.0), "dx": r.f(-50.0, 50.0), "dy": r.f(-50.0, 50.0)})
        }
        6 => json!({"ids": ids.iter().take(3).collect::<Vec<_>>()}),
        7 => json!({"effect": "fade", "start": "afterPrevious", "duration": r.below(5000), "delay": r.below(2000)}),
        _ => json!({"layout": "titleAndContent", "title": "T", "body": "B"}),
    }
}

/// Edits the fuzz leans on, so the people often work on the same slides and shapes at once.
const COMMON: &[&str] = &[
    "slide.new",
    "slide.delete",
    "slide.duplicate",
    "slide.move",
    "shape.insert",
    "shape.move",
    "arrange.nudge",
    "edit.delete",
    "format.bold",
    "text.insert",
    "arrange.bringToFront",
    "arrange.sendToBack",
    "arrange.group",
    "edit.undo",
    "edit.redo",
];

fn fuzz(seed: u64, steps: usize, n: u64) {
    let mut server = Server::default();
    let mut all = people(n);
    let ids: Vec<&'static str> = command_specs().iter().map(|c| c.id).filter(|id| !skipped(id)).collect();
    let mut r = Rng(seed | 1);
    let mut log: VecDeque<String> = VecDeque::new();
    let mut edits = 0usize;
    for step in 0..steps {
        let who = r.below(all.len());
        let ctx = |log: &VecDeque<String>| format!("seed {seed} step {step}; last actions:\n{}", log.iter().cloned().collect::<Vec<_>>().join("\n"));
        match r.below(10) {
            // Someone sends, or gets the next log entry.
            0 | 1 => {
                if all[who].push(&mut server) {
                    log.push_back(format!("{} sends", who + 1));
                }
            }
            2 | 3 => {
                if all[who].deliver_one(&server) {
                    log.push_back(format!("{} receives", who + 1));
                    let problems = all[who].doc().validate();
                    assert!(problems.is_empty(), "after receiving: {problems:?}; {}", ctx(&log));
                }
            }
            // Someone looks at another slide.
            4 => {
                let slides = all[who].doc().slides.len();
                let to = r.below(slides);
                go_to(&mut all[who], to);
            }
            _ => {
                let id = if r.below(2) == 0 { *r.pick(COMMON).unwrap() } else { *r.pick(&ids).unwrap() };
                let p = params(&mut r, &all[who].s);
                log.push_back(format!("{} runs {id} {p}", who + 1));
                let before = all[who].doc().clone();
                let changed = all[who].run(id, &p);
                let after = all[who].doc().clone();
                if changed {
                    edits += 1;
                    let problems = after.validate();
                    assert!(problems.is_empty(), "{problems:?}; {}", ctx(&log));
                    // Commands leave nothing for `normalize` to put right.
                    let mut tidy = (*after).clone();
                    normalize(&mut tidy);
                    let fix = capture(&after, &tidy).unwrap();
                    assert!(fix.is_empty(), "{id} left something to tidy: {}; {}", short(&serde_json::to_value(&fix).unwrap()), ctx(&log));
                    // The op rebuilds the edit exactly.
                    let op = capture(&before, &after).unwrap();
                    let mut rebuilt = (*before).clone();
                    apply(&mut rebuilt, &op).unwrap_or_else(|e| panic!("own op refused: {e}; {}", ctx(&log)));
                    assert!(capture(&rebuilt, &after).unwrap().is_empty(), "the op doesn't rebuild the edit; {}", ctx(&log));
                    // Undo then redo of a fresh edit gets the same presentation back.
                    if !matches!(id, "edit.undo" | "edit.redo") && r.below(4) == 0 {
                        assert!(all[who].run("edit.undo", &json!({})), "nothing to undo; {}", ctx(&log));
                        let undone = canonical(all[who].doc());
                        let was = canonical(&before);
                        assert!(undone == was, "undo didn't restore: {}; {}", first_diff(&was, &undone, ""), ctx(&log));
                        assert!(all[who].run("edit.redo", &json!({})), "nothing to redo; {}", ctx(&log));
                        let redone = canonical(all[who].doc());
                        let now = canonical(&after);
                        assert!(redone == now, "redo didn't reapply: {}; {}", first_diff(&now, &redone, ""), ctx(&log));
                    }
                }
            }
        }
        while log.len() > 30 {
            log.pop_front();
        }
        for p in &all {
            assert!(!p.doc().slides.is_empty(), "no slides left; {}", ctx(&log));
        }
    }
    settle(&mut server, &mut all);
    assert_same(&all, &format!("seed {seed}, {n} people, {steps} steps"));
    for p in &all {
        let problems = p.doc().validate();
        assert!(problems.is_empty(), "{problems:?}; seed {seed}");
    }
    // The shared presentation saves and reopens as .pptx, the same for everyone.
    let mut files = Vec::new();
    for p in all.iter_mut() {
        let data = p.s.execute("file.saveBytes", &json!({"format": "pptx"})).unwrap_or_else(|e| panic!("save: {e}; seed {seed}"));
        let b64 = data["data"].as_str().unwrap_or_default().to_string();
        p.s.execute("file.openBytes", &json!({"name": "shared.pptx", "data": b64})).unwrap_or_else(|e| panic!("reopen: {e}; seed {seed}"));
        let back = p.s.doc().unwrap().doc.clone();
        assert!(back.validate().is_empty(), "reopened copy invalid; seed {seed}");
        let last = p.s.documents().len() - 1;
        p.s.close_document(last);
        p.s.set_active(0);
        assert_eq!(back.slides.len(), p.doc().slides.len(), "pptx slide count; seed {seed}");
        files.push(canonical(&back));
    }
    for (i, f) in files.iter().enumerate().skip(1) {
        assert!(*f == files[0], "pptx of copies 1 and {} differ: {}; seed {seed}", i + 1, first_diff(&files[0], f, ""));
    }
    assert!(edits * 10 >= steps, "only {edits} of {steps} steps edited; seed {seed}");
    eprintln!("collab fuzz seed {seed}: {n} people, {edits} edits, {} ops in the log", server.log.len());
}

fn steps(default: usize) -> usize {
    std::env::var("DECKCRAFT_COLLAB_STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[test]
fn copies_converge_whatever_the_timing() {
    for (seed, n) in [(1, 2), (2, 3), (0xC0_11AB, 3), (42, 2), (7_777_777, 3)] {
        fuzz(seed, steps(300), n);
    }
}

#[test]
#[ignore = "long fuzz; set DECKCRAFT_COLLAB_STEPS and DECKCRAFT_COLLAB_SEEDS"]
fn copies_converge_long() {
    let seeds = std::env::var("DECKCRAFT_COLLAB_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(50u64);
    for seed in 1..=seeds {
        fuzz(seed * 7919, steps(1000), 2 + seed % 2);
    }
}

#[test]
#[ignore = "replays one fuzz seed: DECKCRAFT_COLLAB_SEED, DECKCRAFT_COLLAB_PEOPLE"]
fn copies_converge_one_seed() {
    let seed = std::env::var("DECKCRAFT_COLLAB_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let n = std::env::var("DECKCRAFT_COLLAB_PEOPLE").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    fuzz(seed, steps(1000), n);
}
