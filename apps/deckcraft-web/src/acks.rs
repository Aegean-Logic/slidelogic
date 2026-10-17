//! Save acknowledgements in host mode (`?host=parent&ack=1`), and the checks on the names the
//! page sends.
//!
//! Without acknowledgements, a presentation handed to the parent (`deckcraft:save`) counts as
//! saved at once, even when the parent then fails to store it. With them, every save of the
//! presentation carries an `id` and stays pending until the parent answers `deckcraft:saved { id }`
//! (the presentation as it was handed over is now the saved one; edits made since stay unsaved) or
//! `deckcraft:saveFailed { id, error }` (it stays unsaved and the user is told). No browser types
//! here, so it's tested natively.

/// The presentations `deckcraft:open` accepts, and the formats a save to the page keeps.
pub const PRESENTATION_EXTS: &[&str] = &["pptx", "potx", "ppsx", "deckcraft"];
/// The name a presentation from the page opens under when the page gives none.
pub const DEFAULT_NAME: &str = "Presentation.pptx";

/// Saves still waiting for an answer. Past this the oldest is given up on (as failed), so a parent
/// that never answers can't grow the list without bound.
const MAX_PENDING: usize = 64;
/// The longest file name taken from the parent.
const MAX_NAME_CHARS: usize = 255;
/// The longest error text taken from the parent.
const MAX_ERROR_CHARS: usize = 500;
/// The largest integer a JavaScript number holds exactly.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// The last path component of `path`, if it is a usable file name: no control characters, not
/// empty, `.` or `..`, at most [`MAX_NAME_CHARS`] characters.
pub fn clean_name(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next()?.trim();
    if name.is_empty() || name == "." || name == ".." || name.chars().any(char::is_control) || name.chars().count() > MAX_NAME_CHARS {
        return None;
    }
    Some(name.to_string())
}

/// The extension of `name`, lower-cased ("" when it has none).
pub fn ext(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() => e.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// The name a `deckcraft:open` opens under: the page's name when it is a presentation SlideLogic
/// opens, [`DEFAULT_NAME`] when the page gave none, `None` (ignored) otherwise.
pub fn open_name(given: Option<&str>) -> Option<String> {
    let Some(given) = given else { return Some(DEFAULT_NAME.to_string()) };
    let name = clean_name(given)?;
    PRESENTATION_EXTS.contains(&ext(&name).as_str()).then_some(name)
}

/// The name and format a presentation saves to the page in: its own format when it is one
/// SlideLogic writes (`.pptx`, `.potx`, `.ppsx`, `.deckcraft`), else PowerPoint under the same
/// stem (`Talk.key` → `Talk.pptx`). `None` (an unsaved presentation) saves as `<title>.pptx`.
pub fn save_target(path: Option<&str>, title: &str) -> (String, &'static str) {
    let name = path.and_then(clean_name).unwrap_or_else(|| format!("{}.pptx", clean_name(title).unwrap_or_else(|| "Presentation".into())));
    match ext(&name).as_str() {
        "pptx" => (name, "pptx"),
        "potx" => (name, "potx"),
        "ppsx" => (name, "ppsx"),
        "deckcraft" => (name, "deckcraft"),
        _ => {
            let stem = name.rsplit_once('.').map(|(s, _)| s).filter(|s| !s.is_empty()).unwrap_or(&name);
            (format!("{stem}.pptx"), "pptx")
        }
    }
}

/// The parent's answer to a `deckcraft:save`.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// Stored, under `name` when the parent stored it under another name (a copy, after a
    /// conflict).
    Saved {
        id: u64,
        name: Option<String>,
    },
    Failed {
        id: u64,
        error: String,
    },
}

impl Reply {
    /// The answer in a message from the parent: its `type`, `id`, and `name` or `error`. `None`
    /// for any other message, or one without a valid `id`.
    pub fn parse(kind: &str, id: Option<f64>, name: Option<String>, error: Option<String>) -> Option<Reply> {
        let id = id.filter(|id| id.is_finite() && id.fract() == 0.0 && *id >= 1.0 && *id <= MAX_SAFE_INTEGER)? as u64;
        match kind {
            "deckcraft:saved" => Some(Reply::Saved { id, name: name.as_deref().and_then(clean_name) }),
            "deckcraft:saveFailed" => {
                let error: String = error.unwrap_or_default().trim().chars().filter(|c| !c.is_control()).take(MAX_ERROR_CHARS).collect();
                Some(Reply::Failed { id, error: if error.is_empty() { "the page couldn't store it".into() } else { error } })
            }
            _ => None,
        }
    }
}

/// How a save ended, for the presentation `doc` (its `DocState::uid`).
#[derive(Debug, PartialEq)]
pub enum Settled<T> {
    /// Stored: `handed` (the presentation as handed over) is the saved one, now the file `name`.
    Saved { doc: u64, name: String, handed: T },
    /// Not stored; the presentation stays unsaved.
    Failed { doc: u64, name: String, error: String },
}

struct Pending<T> {
    id: u64,
    doc: u64,
    /// The name it was handed over as.
    name: String,
    handed: T,
}

/// Saves of presentations waiting for the parent's answer. `T` is what was handed over (the
/// presentation's `Arc`), kept to mark it saved once it is stored.
pub struct SaveAcks<T> {
    last_id: u64,
    pending: Vec<Pending<T>>,
    /// Saves given up on before an answer came.
    dropped: Vec<Settled<T>>,
}

impl<T> Default for SaveAcks<T> {
    fn default() -> Self {
        Self { last_id: 0, pending: Vec::new(), dropped: Vec::new() }
    }
}

impl<T> SaveAcks<T> {
    /// The presentation `doc` is being handed to the parent as `name`: the id its answer will carry.
    pub fn begin(&mut self, doc: u64, name: &str, handed: T) -> u64 {
        self.last_id = self.last_id.saturating_add(1);
        if self.pending.len() >= MAX_PENDING {
            let oldest = self.pending.remove(0);
            self.dropped.push(Settled::Failed { doc: oldest.doc, name: oldest.name, error: "the page didn't answer".into() });
        }
        self.pending.push(Pending { id: self.last_id, doc, name: name.to_string(), handed });
        self.last_id
    }

    /// The save `id` never reached the parent.
    pub fn forget(&mut self, id: u64) {
        self.pending.retain(|p| p.id != id);
    }

    /// Whether a save of `doc` is still waiting for the parent: until it answers, the
    /// presentation isn't safely stored.
    pub fn unconfirmed(&self, doc: u64) -> bool {
        self.pending.iter().any(|p| p.doc == doc)
    }

    /// Saves given up on since the last call.
    pub fn take_dropped(&mut self) -> Vec<Settled<T>> {
        std::mem::take(&mut self.dropped)
    }

    /// Apply the parent's answer. `None` for an answer to no save waiting (unknown, or already
    /// superseded by a newer save of the same presentation that was stored).
    pub fn apply(&mut self, reply: Reply) -> Option<Settled<T>> {
        match reply {
            Reply::Saved { id, name } => {
                let index = self.pending.iter().position(|p| p.id == id)?;
                let p = self.pending.remove(index);
                // Earlier saves of the same presentation are superseded: the page has a newer copy,
                // so a late answer to one of them mustn't change what counts as saved.
                self.pending.retain(|q| !(q.id < id && q.doc == p.doc));
                // The page's own name only when it is the same kind of file.
                let name = name.filter(|n| ext(n) == ext(&p.name)).unwrap_or(p.name);
                Some(Settled::Saved { doc: p.doc, name, handed: p.handed })
            }
            Reply::Failed { id, error } => {
                let index = self.pending.iter().position(|p| p.id == id)?;
                let p = self.pending.remove(index);
                Some(Settled::Failed { doc: p.doc, name: p.name, error })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    #[test]
    fn names_from_the_page_are_checked() {
        assert_eq!(clean_name("Team/Q3 review.pptx").as_deref(), Some("Q3 review.pptx"));
        assert_eq!(clean_name("C:\\decks\\a.pptx").as_deref(), Some("a.pptx"));
        for bad in ["", "  ", "..", "a/", "bad\u{0}.pptx"] {
            assert_eq!(clean_name(bad), None, "{bad:?}");
        }
        assert_eq!(clean_name(&"x".repeat(300)), None);
        assert_eq!(open_name(None).as_deref(), Some(DEFAULT_NAME));
        assert_eq!(open_name(Some("Talk.PPTX")).as_deref(), Some("Talk.PPTX"));
        assert_eq!(open_name(Some("deck.deckcraft")).as_deref(), Some("deck.deckcraft"));
        assert_eq!(open_name(Some("notes.txt")), None);
        assert_eq!(open_name(Some(".pptx")), None);
    }

    #[test]
    fn saves_keep_a_format_slidelogic_writes() {
        assert_eq!(save_target(Some("Talk.pptx"), "Talk"), ("Talk.pptx".into(), "pptx"));
        assert_eq!(save_target(Some("Brand.POTX"), "Brand"), ("Brand.POTX".into(), "potx"));
        assert_eq!(save_target(Some("Loop.ppsx"), "Loop"), ("Loop.ppsx".into(), "ppsx"));
        assert_eq!(save_target(Some("Mine.deckcraft"), "Mine"), ("Mine.deckcraft".into(), "deckcraft"));
        assert_eq!(save_target(Some("Old.ppt"), "Old"), ("Old.pptx".into(), "pptx"));
        assert_eq!(save_target(None, "Presentation1"), ("Presentation1.pptx".into(), "pptx"));
        assert_eq!(save_target(None, ""), ("Presentation.pptx".into(), "pptx"));
    }

    #[test]
    fn replies_need_a_valid_id() {
        assert_eq!(Reply::parse("deckcraft:saved", Some(3.0), None, None), Some(Reply::Saved { id: 3, name: None }));
        for id in [None, Some(0.0), Some(-1.0), Some(1.5), Some(f64::NAN), Some(1e300)] {
            assert_eq!(Reply::parse("deckcraft:saved", id, None, None), None, "{id:?}");
        }
        assert_eq!(Reply::parse("deckcraft:other", Some(1.0), None, None), None);
        let Some(Reply::Failed { error, .. }) = Reply::parse("deckcraft:saveFailed", Some(1.0), None, Some("  \n".into())) else { panic!() };
        assert_eq!(error, "the page couldn't store it");
    }

    #[test]
    fn a_stored_save_settles_older_ones_of_the_same_presentation() {
        let mut acks = SaveAcks::default();
        let a = acks.begin(7, "Talk.pptx", "first");
        let b = acks.begin(7, "Talk.pptx", "second");
        let other = acks.begin(9, "Other.pptx", "other");
        assert!(acks.unconfirmed(7));
        assert_eq!(acks.apply(Reply::Saved { id: b, name: None }), Some(Settled::Saved { doc: 7, name: "Talk.pptx".into(), handed: "second" }));
        assert!(!acks.unconfirmed(7), "the older save is superseded");
        assert_eq!(acks.apply(Reply::Failed { id: a, error: "late".into() }), None, "a late failure of it changes nothing");
        assert!(acks.unconfirmed(9));
        assert_eq!(
            acks.apply(Reply::Failed { id: other, error: "no room".into() }),
            Some(Settled::Failed { doc: 9, name: "Other.pptx".into(), error: "no room".into() })
        );
        assert_eq!(acks.apply(Reply::Saved { id: 99, name: None }), None, "unknown ids are ignored");
    }

    #[test]
    fn the_pages_name_is_taken_only_for_the_same_kind_of_file() {
        let mut acks = SaveAcks::default();
        let a = acks.begin(1, "Talk.pptx", ());
        let Some(Settled::Saved { name, .. }) = acks.apply(Reply::Saved { id: a, name: Some("Talk (2).pptx".into()) }) else { panic!() };
        assert_eq!(name, "Talk (2).pptx");
        let b = acks.begin(1, "Talk.pptx", ());
        let Some(Settled::Saved { name, .. }) = acks.apply(Reply::Saved { id: b, name: Some("Talk.pdf".into()) }) else { panic!() };
        assert_eq!(name, "Talk.pptx");
    }

    #[test]
    fn a_parent_that_never_answers_cant_grow_the_list() {
        let mut acks = SaveAcks::default();
        for _ in 0..MAX_PENDING + 3 {
            acks.begin(1, "Talk.pptx", ());
        }
        assert_eq!(acks.pending.len(), MAX_PENDING);
        assert_eq!(acks.take_dropped().len(), 3);
        assert!(acks.take_dropped().is_empty());
    }

    #[test]
    fn a_save_that_never_left_is_forgotten() {
        let mut acks = SaveAcks::default();
        let a = acks.begin(1, "Talk.pptx", ());
        acks.forget(a);
        assert!(!acks.unconfirmed(1));
    }
}
