//! The `postMessage` bridge to a same-origin parent page (`?host=parent`), for pages that embed
//! SlideLogic in an `<iframe>` (such as the Nextcloud app). Messages are plain objects with a
//! `type`:
//!
//! - `deckcraft:ready` (to the parent): `{ version, ack }`, the app is listening; send a file now.
//! - `deckcraft:open` (from the parent): `{ name, bytes }`, bytes an `ArrayBuffer` or
//!   `Uint8Array`. It opens like a presentation picked with File › Open, and Save goes back to it.
//!   `name` must be a presentation SlideLogic opens ([`acks::PRESENTATION_EXTS`]); without one it
//!   opens as `Presentation.pptx`. Empty bytes open a new blank presentation that saves under that
//!   name, so a page can create a file and open it straight away.
//! - `deckcraft:save` (to the parent): `{ name, bytes, copy }` (`bytes` a transferred
//!   `ArrayBuffer`). Save sends the presentation in its own format (.pptx, .potx, .ppsx,
//!   .deckcraft; anything else as .pptx under the same name) with `copy: false`. Save As, Export
//!   and Print send `copy: true`: files to store next to it that don't change what counts as saved.
//! - `deckcraft:dirty` (to the parent): `{ dirty }` whenever unsaved changes appear or go away.
//! - `deckcraft:failed` (to the parent): `{ error }` when the app couldn't start.
//!
//! With `&ack=1` the parent confirms every save of the presentation (`acks`): `deckcraft:save`
//! also carries an `id`, and the parent answers `deckcraft:saved { id, name? }` once it stored the
//! file or `deckcraft:saveFailed { id, error }` when it couldn't. Until then the presentation is
//! reported dirty; a failure leaves it unsaved and is shown to the user. `deckcraft:ready` says
//! `{ ack: true }` when this is on, so a parent can tell a SlideLogic that ignores the flag.
//! Without it, a presentation counts as saved as soon as it's handed over.
//!
//! Co-editing (the page runs the session with the server; [`collab_msg`] checks what it sends):
//!
//! - `deckcraft:ready` says `{ collab: n }`: this build can co-edit, with ops as
//!   `collab::PROTOCOL` n means them (the page joins with it: a session keeps to one).
//! - `deckcraft:collab:start` (from the parent): `{ client (or clientId), seq, readOnly?, user?,
//!   color?, state? }`, after `deckcraft:open`: the presentation just opened is the shared one as
//!   of log position `seq`. With `state` (a checkpoint's snapshot, `Session::collab_load_snapshot`)
//!   the snapshot is, and replaces the file just opened (a .pptx read back numbers its slides
//!   afresh). `readOnly`: this person can follow but not edit.
//! - `deckcraft:collab:entries` (from the parent): `{ json }`, the server's log entries in order
//!   (JSON text of `collab::Incoming`s).
//! - `deckcraft:collab:op` (to the parent): `{ json }`, this person's next op for the server
//!   (`collab::Outgoing`). The next comes once this one is back in the log.
//! - `deckcraft:collab:checkpoint` (from the parent): `{ id }`; answered with
//!   `deckcraft:collab:checkpointData { id, seq, name, bytes, state }`: the shared presentation
//!   (without this person's unconfirmed edits) in the opened file's format, its log position, and
//!   its state snapshot (text) for people who join from that save.
//! - `deckcraft:collab:selection` (to the parent): `{ json }`, where this person is (`{slide,
//!   shapes}`, `Session::collab_presence`) whenever it changes; `deckcraft:collab:presence` (from
//!   the parent): `{ json }`, the others' (`[{client, name, color, slide, shapes}]`), drawn on the
//!   slide and its thumbnail.
//! - `deckcraft:collab:stop` (from the parent) ends sharing; `deckcraft:collab:error` (to the
//!   parent) `{ error }` says this copy can't follow the log and must rejoin.
//!
//! Co-editing messages are handled as they arrive, not at the next frame, so a hidden tab (which
//! draws none) keeps up with the others and answers checkpoint requests ([`Bridge::attach`]).
//! Not before the app's first frame, though: until then egui has no fonts, and opening the
//! presentation would panic. A tab opened in the background joins when it's first shown.
//!
//! Sharing (the Share dialog's cloud is the page, which has the user's Nextcloud session):
//!
//! - `deckcraft:cloud` (to the parent): `{ id, op, params }`, an operation of the Share dialog
//!   (`deckcraft_ui_egui::share`; `params` as JSON text).
//! - `deckcraft:cloud:reply` (from the parent): `{ id, ok, json?, error? }`, its answer (`json` as
//!   JSON text when `ok`, else `error`).
//! - `deckcraft:openDesktop` (to the parent, offered with `desktop=1` in the URL): Open in Desktop
//!   App was chosen; the page opens the presentation in SlideLogic on this computer (a
//!   `slidelogic://open` link).
//!
//! Only the parent window on the page's own origin is heard or answered. In host mode the app
//! starts with no presentation (the page sends one), and the parent guards leaving the page.
//!
//! Adding a message: a [`FromPage`] variant, its case in [`Bridge::parse`] and in
//! [`Bridge::handle`].

use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use deckcraft_engine::DocState;
use deckcraft_engine::model::Presentation;
use deckcraft_ui_egui::cloud::{Cloud, CloudInbox};
use deckcraft_ui_egui::{HostSave, Services, SlideApp};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use crate::acks::{self, Reply, SaveAcks, Settled};
use crate::collab_msg::{self, Fields};

/// The longest user name taken from `?author=` (it signs new comments).
const MAX_AUTHOR_CHARS: usize = 200;
/// The most save answers held between two frames; more are dropped.
const MAX_REPLIES: usize = 256;
/// The most open requests held between two frames; more are dropped.
const MAX_OPENS: usize = 4;
/// The largest presentation the page can send.
const MAX_OPEN_BYTES: u32 = 500 << 20;
/// The longest strings read from a message, in UTF-16 units: longer ones are ignored unread.
const MAX_TYPE_UNITS: u32 = 64;
const MAX_NAME_UNITS: u32 = 4096;
const MAX_ERROR_UNITS: u32 = 64 * 1024;
/// Log batches, state snapshots and presence lists (JSON text).
const MAX_JSON_UNITS: u32 = collab_msg::MAX_JSON_BYTES as u32;
/// A person's name.
const MAX_SHORT_UNITS: u32 = 1024;
/// The most co-editing messages held for the app; past this the page has to rejoin.
const MAX_COLLAB_QUEUE: usize = 4096;
/// The largest answer to a sharing request taken from the page (JSON text).
const MAX_CLOUD_REPLY_UNITS: u32 = 4 << 20;
/// The most sharing answers held between two frames.
const MAX_CLOUD_REPLIES: usize = 256;

/// A message from the page, checked.
enum FromPage {
    /// `deckcraft:open`: a presentation to open (its name checked by [`acks::open_name`]).
    Open { name: String, bytes: Vec<u8> },
    /// `deckcraft:saved` / `deckcraft:saveFailed` (`&ack=1` only).
    Reply(Reply),
    /// `deckcraft:collab:*`.
    Collab(collab_msg::FromPage),
    /// `deckcraft:cloud:reply`: the answer to the Share dialog's request `id`.
    CloudReply { id: u64, answer: Result<serde_json::Value, String> },
}

#[derive(Clone)]
pub struct Bridge {
    window: web_sys::Window,
    parent: web_sys::Window,
    origin: String,
    /// `&ack=1`: saves the parent hasn't answered yet; `None` when the parent doesn't answer.
    acks: Option<Rc<RefCell<SaveAcks<Arc<Presentation>>>>>,
    /// Without `&ack=1`: presentations handed to the parent since the last frame, and the names
    /// they were stored as.
    stored: Rc<RefCell<Vec<(u64, String)>>>,
    /// Presentations the page sent since the last frame.
    opens: Rc<RefCell<Vec<(String, Vec<u8>)>>>,
    /// The parent's answers since the last frame.
    replies: Rc<RefCell<Vec<Reply>>>,
    /// The unsaved-changes state last reported to the parent.
    reported: Rc<Cell<Option<bool>>>,
    /// The app has drawn its first frame: before it, egui has no fonts and opening a
    /// presentation would panic, so co-editing messages wait for that frame.
    framed: Rc<Cell<bool>>,
    /// Co-editing messages waiting for the app.
    collab: Rc<RefCell<VecDeque<collab_msg::FromPage>>>,
    /// The app, for handling co-editing messages as they arrive ([`Bridge::attach`]).
    app: Rc<RefCell<Option<Weak<RefCell<SlideApp>>>>>,
    /// Where this person is, as last told to the parent.
    position: Rc<RefCell<String>>,
    ctx: Rc<RefCell<Option<egui::Context>>>,
    /// The page's answers to the Share dialog's requests.
    cloud: CloudInbox,
}

impl Bridge {
    /// `None` when the page isn't framed (there is no parent to talk to). `ack`: the parent
    /// confirms saves (`&ack=1`).
    pub fn new(ack: bool) -> Option<Self> {
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let origin = window.location().origin().ok()?;
        Some(Self {
            window,
            parent,
            origin,
            acks: ack.then(Rc::default),
            stored: Rc::default(),
            opens: Rc::default(),
            replies: Rc::default(),
            reported: Rc::default(),
            framed: Rc::default(),
            collab: Rc::default(),
            app: Rc::default(),
            position: Rc::default(),
            ctx: Rc::default(),
            cloud: CloudInbox::default(),
        })
    }

    /// Save to the parent instead of downloading, send it Save As, Export and Print copies, and
    /// hear presentations from it.
    pub fn connect(&self, services: &mut Services, ctx: egui::Context) {
        *self.ctx.borrow_mut() = Some(ctx.clone());
        self.listen(ctx);
        let bridge = self.clone();
        services.save_to_host = Some(Box::new(move |st: &DocState| bridge.post_presentation(st)));
        let bridge = self.clone();
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| {
            let name = acks::clean_name(name).unwrap_or_else(|| acks::DEFAULT_NAME.to_string());
            if let Err(e) = bridge.post_save(&name, bytes, true, None) {
                log::error!("couldn't hand {name} to the page: {e}");
            }
        }));
        // The page is also the Share dialog's cloud: it has the user's Nextcloud session.
        let bridge = self.clone();
        services.cloud = Some(Cloud::sharing(
            Box::new(move |id, op, params| {
                let params = params.to_string();
                // Ids stay far below 2^53, so a JavaScript number holds them exactly.
                bridge.send(&[
                    ("type", "deckcraft:cloud".into()),
                    ("id", JsValue::from_f64(id as f64)),
                    ("op", op.into()),
                    ("params", params.as_str().into()),
                ]);
            }),
            self.cloud.clone(),
        ));
    }

    /// `desktop=1`: the page can open the presentation in the desktop app (`deckcraft:openDesktop`,
    /// to the page): the title bar offers Open in Desktop App.
    pub fn offer_desktop(&self, services: &mut Services) {
        let bridge = self.clone();
        services.open_in_desktop = Some(Box::new(move || {
            bridge.send(&[("type", "deckcraft:openDesktop".into())]);
        }));
    }

    /// `?author=`: the host's name for the user signs new comments.
    pub fn set_author(app: &mut SlideApp, author: &str) {
        let author: String = author.trim().chars().filter(|c| !c.is_control()).take(MAX_AUTHOR_CHARS).collect();
        if !author.is_empty() {
            app.session.prefs.author = author;
        }
    }

    pub fn post_ready(&self) {
        self.send(&[
            ("type", "deckcraft:ready".into()),
            ("version", env!("CARGO_PKG_VERSION").into()),
            ("ack", self.acks.is_some().into()),
            // The co-editing protocol (truthy for pages that only ask whether it co-edits).
            ("collab", JsValue::from_f64(f64::from(deckcraft_engine::collab::PROTOCOL))),
        ]);
    }

    pub fn post_failed(&self, error: &str) {
        self.send(&[("type", "deckcraft:failed".into()), ("error", error.into())]);
    }

    /// Handle co-editing messages as they arrive rather than at the next frame, so a hidden tab
    /// (which draws none) keeps up with the others and answers checkpoint requests.
    pub fn attach(&self, app: &Rc<RefCell<SlideApp>>) {
        *self.app.borrow_mut() = Some(Rc::downgrade(app));
    }

    /// Co-editing messages now, if the app is free (presentations the page sent first are opened
    /// first: a session starts on the file that came just before it).
    fn sync_collab_now(&self) {
        if !self.framed.get() {
            return;
        }
        let Some(app) = self.app.borrow().as_ref().and_then(Weak::upgrade) else { return };
        let Ok(mut app) = app.try_borrow_mut() else { return };
        self.open_pending(&mut app);
        self.sync_collab(&mut app);
    }

    fn open_pending(&self, app: &mut SlideApp) {
        let opens = std::mem::take(&mut *self.opens.borrow_mut());
        for (name, bytes) in opens {
            open(app, &name, &bytes);
            self.repaint();
        }
    }

    fn repaint(&self) {
        if let Some(ctx) = self.ctx.borrow().as_ref() {
            ctx.request_repaint();
        }
    }

    /// After each frame: open what the page sent, settle saves it answered (with `&ack=1`) or
    /// took (without), carry co-editing on, and tell the page when unsaved changes appear or go
    /// away.
    pub fn sync(&self, app: &mut SlideApp, ctx: &egui::Context) {
        self.framed.set(true);
        self.open_pending(app);
        self.sync_collab(app);
        if let Some(acks) = &self.acks {
            let replies = std::mem::take(&mut *self.replies.borrow_mut());
            let mut settled: Vec<Settled<Arc<Presentation>>> = Vec::new();
            {
                let mut acks = acks.borrow_mut();
                settled.extend(replies.into_iter().filter_map(|r| acks.apply(r)));
                settled.extend(acks.take_dropped());
            }
            for s in settled {
                match s {
                    Settled::Saved { doc, name, handed } => {
                        if let Some(st) = app.session.document_mut(doc) {
                            st.saved_doc = handed;
                            st.path = Some(name);
                            app.set_status("Saved");
                        }
                    }
                    Settled::Failed { name, error, .. } => app.notify(format!("Couldn't save {name}: {error}")),
                }
                ctx.request_repaint();
            }
        } else {
            let stored = std::mem::take(&mut *self.stored.borrow_mut());
            for (doc, name) in stored {
                if let Some(st) = app.session.document_mut(doc) {
                    st.path = Some(name);
                }
            }
        }
        let unconfirmed = |uid: u64| self.acks.as_ref().is_some_and(|a| a.borrow().unconfirmed(uid));
        let dirty = app.session.documents().iter().any(|d| d.is_dirty() || unconfirmed(d.uid));
        if self.reported.get() != Some(dirty) && self.send(&[("type", "deckcraft:dirty".into()), ("dirty", dirty.into())]) {
            self.reported.set(Some(dirty));
        }
    }

    /// Co-editing: the page's messages in order, then this person's place and next op.
    fn sync_collab(&self, app: &mut SlideApp) {
        let queued: Vec<collab_msg::FromPage> = self.collab.borrow_mut().drain(..).collect();
        let mut changed = false;
        for msg in queued {
            match msg {
                collab_msg::FromPage::Start { client, seq, read_only, user, state } => {
                    // A checkpoint's snapshot is the shared presentation exactly; the .pptx read
                    // back would number its slides afresh.
                    let loaded = match &state {
                        Some(snapshot) => app.session.collab_load_snapshot(snapshot),
                        None => Ok(()),
                    };
                    match loaded.and_then(|()| app.session.collab_start(client, seq)) {
                        Ok(()) => {
                            app.session.collab_set_read_only(read_only);
                            self.position.borrow_mut().clear();
                            log::info!("co-editing as client {client} ({}) from log position {seq}", user.as_deref().unwrap_or("?"));
                        }
                        Err(e) => self.post_collab_error(&e.to_string()),
                    }
                    changed = true;
                }
                collab_msg::FromPage::Entries(entries) => {
                    for entry in entries {
                        if let Err(e) = app.session.collab_receive(entry) {
                            app.session.collab_stop();
                            self.post_collab_error(&e.to_string());
                            break;
                        }
                        changed = true;
                    }
                }
                collab_msg::FromPage::Checkpoint { id } => self.post_checkpoint(app, id),
                collab_msg::FromPage::Stop => {
                    app.session.collab_stop();
                    changed = true;
                }
                collab_msg::FromPage::Presence(people) => {
                    app.session.collab_set_others(people);
                    changed = true;
                }
            }
        }
        if !app.session.collab_active() {
            return;
        }
        let json = app.session.collab_presence().to_string();
        if *self.position.borrow() != json && self.send(&[("type", "deckcraft:collab:selection".into()), ("json", json.as_str().into())]) {
            *self.position.borrow_mut() = json;
        }
        if let Some(out) = app.session.collab_outgoing() {
            match serde_json::to_string(&out) {
                Ok(json) => {
                    self.send(&[("type", "deckcraft:collab:op".into()), ("json", json.into())]);
                }
                Err(e) => self.post_collab_error(&e.to_string()),
            }
        }
        if let Some(notice) = app.session.collab_take_notice() {
            app.notify(notice);
            changed = true;
        }
        if changed {
            self.repaint();
        }
    }

    /// The shared presentation as everyone has it, for the page to save as the session's
    /// checkpoint: in the opened file's format, with its state snapshot.
    fn post_checkpoint(&self, app: &mut SlideApp, id: u64) {
        let target =
            app.session.documents().iter().find(|d| app.session.collab_shares(d.uid)).map(|d| acks::save_target(d.path.as_deref(), &d.title()));
        let data = match (target, app.session.collab_snapshot(), app.session.collab_confirmed()) {
            (Some((name, format)), Some(snapshot), Some((_, doc))) => {
                Some(snapshot.and_then(|(seq, state)| Ok((seq, state, deckcraft_engine::cmd::file::save_bytes(doc, format)?, name))))
            }
            _ => None,
        };
        let (seq, state, bytes, name) = match data {
            Some(Ok(data)) => data,
            Some(Err(e)) => {
                // Not a reason to rejoin: the page's save fails and is tried again later.
                log::error!("co-editing: couldn't write the shared presentation: {e}");
                app.notify(format!("Couldn't save the shared presentation: {e}"));
                return;
            }
            None => {
                self.post_collab_error("no shared presentation to save");
                return;
            }
        };
        let buffer = js_sys::Uint8Array::from(bytes.as_slice()).buffer();
        let fields = [
            ("type", "deckcraft:collab:checkpointData".into()),
            ("id", JsValue::from_f64(id as f64)),
            ("seq", JsValue::from_f64(seq as f64)),
            ("name", name.as_str().into()),
            ("state", state.as_str().into()),
            ("bytes", buffer.clone().into()),
        ];
        if let Some(m) = message(&fields)
            && self.parent.post_message_with_transfer(&m, &self.origin, &js_sys::Array::of1(&buffer)).is_err()
        {
            log::error!("couldn't hand the checkpoint to the page");
        }
    }

    fn post_collab_error(&self, error: &str) {
        log::error!("co-editing: {error}");
        self.send(&[("type", "deckcraft:collab:error".into()), ("error", error.into())]);
    }

    /// The app's `save_to_host`: hand the presentation to the parent in the format of the file it
    /// came from.
    fn post_presentation(&self, st: &DocState) -> Result<HostSave, String> {
        let (name, format) = acks::save_target(st.path.as_deref(), &st.title());
        let bytes = deckcraft_engine::cmd::file::save_bytes(&st.doc, format).map_err(|e| e.to_string())?;
        let id = self.acks.as_ref().map(|acks| acks.borrow_mut().begin(st.uid, &name, st.doc.clone()));
        if let Err(e) = self.post_save(&name, &bytes, false, id) {
            if let (Some(id), Some(acks)) = (id, &self.acks) {
                acks.borrow_mut().forget(id);
            }
            return Err(e);
        }
        if id.is_some() {
            return Ok(HostSave::Pending);
        }
        self.stored.borrow_mut().push((st.uid, name));
        Ok(HostSave::Saved)
    }

    /// Post `deckcraft:save`; an error means it didn't get to the parent.
    fn post_save(&self, name: &str, bytes: &[u8], copy: bool, id: Option<u64>) -> Result<(), String> {
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        let mut fields = vec![("type", "deckcraft:save".into()), ("name", name.into()), ("bytes", buffer.clone().into()), ("copy", copy.into())];
        if let Some(id) = id {
            // Ids stay far below 2^53, so a JavaScript number holds them exactly.
            fields.push(("id", JsValue::from_f64(id as f64)));
        }
        let message = message(&fields).ok_or_else(|| "couldn't build the message".to_string())?;
        self.parent
            .post_message_with_transfer(&message, &self.origin, &js_sys::Array::of1(&buffer))
            .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
    }

    fn listen(&self, ctx: egui::Context) {
        let bridge = self.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if event.origin() != bridge.origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &bridge.parent)) {
                return;
            }
            if let Some(msg) = bridge.parse(&event.data()) {
                bridge.handle(msg);
                ctx.request_repaint();
            }
        });
        if self.window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
            // The listener lives as long as the page.
            on_message.forget();
        }
    }

    /// The page's message, checked; `None` for anything this bridge doesn't take.
    fn parse(&self, data: &JsValue) -> Option<FromPage> {
        let kind = string(data, "type", MAX_TYPE_UNITS)?;
        match kind.as_str() {
            "deckcraft:open" => {
                let given = string(data, "name", MAX_NAME_UNITS);
                // A name that is there but isn't a short string (not one left out) is refused.
                if given.is_none() && present(data, "name") {
                    log::warn!("deckcraft:open: `name` must be a string of at most {MAX_NAME_UNITS} characters");
                    return None;
                }
                let Some(name) = acks::open_name(given.as_deref()) else {
                    log::warn!("deckcraft:open: {given:?} isn't a presentation SlideLogic opens");
                    return None;
                };
                let bytes = bytes(data, MAX_OPEN_BYTES)?;
                Some(FromPage::Open { name, bytes })
            }
            "deckcraft:saved" | "deckcraft:saveFailed" if self.acks.is_some() => {
                Reply::parse(&kind, number(data, "id"), string(data, "name", MAX_NAME_UNITS), string(data, "error", MAX_ERROR_UNITS))
                    .map(FromPage::Reply)
            }
            "deckcraft:cloud:reply" => {
                let id = number(data, "id").filter(|v| v.is_finite() && *v >= 0.0 && v.fract() == 0.0 && *v <= 9_007_199_254_740_991.0)?;
                let ok = js_sys::Reflect::get(data, &JsValue::from_str("ok")).ok().and_then(|v| v.as_bool()).unwrap_or(false);
                let answer = if ok {
                    match string(data, "json", MAX_CLOUD_REPLY_UNITS) {
                        Some(text) => serde_json::from_str(&text).map_err(|e| e.to_string()),
                        None if present(data, "json") => Err("the answer was too large".to_string()),
                        None => Ok(serde_json::Value::Null),
                    }
                } else {
                    Err(string(data, "error", MAX_ERROR_UNITS).unwrap_or_else(|| "Nextcloud didn't answer".into()).chars().take(500).collect())
                };
                Some(FromPage::CloudReply { id: id as u64, answer })
            }
            _ if kind.starts_with("deckcraft:collab:") => {
                let flag = |key: &str| js_sys::Reflect::get(data, &JsValue::from_str(key)).ok().and_then(|v| v.as_bool());
                let fields = Fields {
                    client: number(data, "client"),
                    client_id: number(data, "clientId"),
                    seq: number(data, "seq"),
                    id: number(data, "id"),
                    read_only: flag("readOnly"),
                    json: string(data, "json", MAX_JSON_UNITS),
                    user: string(data, "user", MAX_SHORT_UNITS),
                    state: string(data, "state", MAX_JSON_UNITS),
                };
                if present(data, "state") && fields.state.is_none() {
                    self.post_collab_error(&format!("{kind}: `state` must be text of at most {MAX_JSON_UNITS} characters"));
                    return None;
                }
                match collab_msg::parse(&kind, fields)? {
                    Ok(msg) => Some(FromPage::Collab(msg)),
                    Err(e) => {
                        self.post_collab_error(&e);
                        None
                    }
                }
            }
            _ => None,
        }
    }

    fn handle(&self, msg: FromPage) {
        match msg {
            FromPage::Open { name, bytes } => {
                let mut opens = self.opens.borrow_mut();
                if opens.len() < MAX_OPENS {
                    opens.push((name, bytes));
                } else {
                    log::warn!("deckcraft:open: too many files at once; {name} dropped");
                }
            }
            FromPage::Reply(reply) => {
                let mut replies = self.replies.borrow_mut();
                if replies.len() < MAX_REPLIES {
                    replies.push(reply);
                }
            }
            FromPage::CloudReply { id, answer } => {
                {
                    let mut answers = self.cloud.lock().unwrap_or_else(|e| e.into_inner());
                    if answers.len() < MAX_CLOUD_REPLIES {
                        answers.push((id, answer));
                    }
                }
                if let Some(ctx) = self.ctx.borrow().as_ref() {
                    ctx.request_repaint();
                }
            }
            FromPage::Collab(msg) => {
                let full = {
                    let mut queue = self.collab.borrow_mut();
                    let full = queue.len() >= MAX_COLLAB_QUEUE;
                    if !full {
                        queue.push_back(msg);
                    }
                    full
                };
                if full {
                    self.post_collab_error("too many messages waiting; rejoin");
                }
                self.sync_collab_now();
            }
        }
    }

    /// Post a message to the parent; false if it couldn't be sent.
    fn send(&self, fields: &[(&str, JsValue)]) -> bool {
        message(fields).is_some_and(|m| self.parent.post_message(&m, &self.origin).is_ok())
    }
}

/// Open the page's presentation `name` (a new blank one for empty `bytes`); Save goes back to
/// `name`.
fn open(app: &mut SlideApp, name: &str, bytes: &[u8]) {
    let before: HashSet<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    if bytes.is_empty() {
        if let Err(e) = app.run("file.new", serde_json::json!({})) {
            app.notify(format!("Couldn't create {name}: {e}"));
        }
    } else {
        app.open_or_insert_bytes(name, bytes);
    }
    // The presentation it opened, if it did: the file it came from is where Save goes.
    let opened = app.session.documents().iter().find(|d| !before.contains(&d.uid)).map(|d| d.uid);
    if let Some(st) = opened.and_then(|uid| app.session.document_mut(uid)) {
        st.path = Some(name.to_string());
    }
}

fn message(fields: &[(&str, JsValue)]) -> Option<js_sys::Object> {
    let m = js_sys::Object::new();
    for (key, value) in fields {
        js_sys::Reflect::set(&m, &JsValue::from_str(key), value).ok()?;
    }
    Some(m)
}

/// The string field `key`, if it is one of at most `max_units` UTF-16 units.
fn string(data: &JsValue, key: &str, max_units: u32) -> Option<String> {
    let value = js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?;
    let text = value.dyn_ref::<js_sys::JsString>()?;
    if text.length() > max_units {
        return None;
    }
    text.as_string()
}

/// Whether the field `key` is there (neither missing, `undefined` nor `null`).
fn present(data: &JsValue, key: &str) -> bool {
    js_sys::Reflect::get(data, &JsValue::from_str(key)).is_ok_and(|v| !v.is_undefined() && !v.is_null())
}

fn number(data: &JsValue, key: &str) -> Option<f64> {
    js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_f64()
}

/// `bytes` as an `ArrayBuffer` or `Uint8Array` of at most `max` bytes.
fn bytes(data: &JsValue, max: u32) -> Option<Vec<u8>> {
    let value = js_sys::Reflect::get(data, &JsValue::from_str("bytes")).ok()?;
    let array = if let Some(array) = value.dyn_ref::<js_sys::Uint8Array>() {
        array.clone()
    } else if value.is_instance_of::<js_sys::ArrayBuffer>() {
        js_sys::Uint8Array::new(&value)
    } else {
        log::warn!("deckcraft:open: `bytes` must be an ArrayBuffer or a Uint8Array");
        return None;
    };
    if array.length() > max {
        log::warn!("deckcraft:open: the presentation is larger than {} MB; not opened", max >> 20);
        return None;
    }
    Some(array.to_vec())
}
