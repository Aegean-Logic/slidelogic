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
//! Only the parent window on the page's own origin is heard or answered. In host mode the app
//! starts with no presentation (the page sends one), and the parent guards leaving the page.
//!
//! Adding a message: a [`FromPage`] variant, its case in [`Bridge::parse`] and in
//! [`Bridge::handle`].

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use deckcraft_engine::DocState;
use deckcraft_engine::model::Presentation;
use deckcraft_ui_egui::{HostSave, Services, SlideApp};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use crate::acks::{self, Reply, SaveAcks, Settled};

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

/// A message from the page, checked.
enum FromPage {
    /// `deckcraft:open`: a presentation to open (its name checked by [`acks::open_name`]).
    Open { name: String, bytes: Vec<u8> },
    /// `deckcraft:saved` / `deckcraft:saveFailed` (`&ack=1` only).
    Reply(Reply),
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
        })
    }

    /// Save to the parent instead of downloading, send it Save As, Export and Print copies, and
    /// hear presentations from it.
    pub fn connect(&self, services: &mut Services, ctx: egui::Context) {
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
    }

    /// `?author=`: the host's name for the user signs new comments.
    pub fn set_author(app: &mut SlideApp, author: &str) {
        let author: String = author.trim().chars().filter(|c| !c.is_control()).take(MAX_AUTHOR_CHARS).collect();
        if !author.is_empty() {
            app.session.prefs.author = author;
        }
    }

    pub fn post_ready(&self) {
        self.send(&[("type", "deckcraft:ready".into()), ("version", env!("CARGO_PKG_VERSION").into()), ("ack", self.acks.is_some().into())]);
    }

    pub fn post_failed(&self, error: &str) {
        self.send(&[("type", "deckcraft:failed".into()), ("error", error.into())]);
    }

    /// After each frame: open what the page sent, settle saves it answered (with `&ack=1`) or
    /// took (without), and tell it when unsaved changes appear or go away.
    pub fn sync(&self, app: &mut SlideApp, ctx: &egui::Context) {
        let opens = std::mem::take(&mut *self.opens.borrow_mut());
        for (name, bytes) in opens {
            open(app, &name, &bytes);
            ctx.request_repaint();
        }
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
