//! The desktop's Nextcloud end to end, against a fake Nextcloud on a local port
//! ([`super::fake_server`]): listing and opening files, sending saves back with ETags, conflicts,
//! Save As, and co-editing a presentation with someone in the browser (a second engine session
//! standing in for the browser's SlideLogic) through the app's sequencer, state snapshots and
//! checkpoints included. Each presentation is told apart by its first slide's notes.

use std::time::{Duration, Instant};

use deckcraft_engine::collab::{Incoming, PROTOCOL};
use deckcraft_engine::{DocState, Session};
use deckcraft_ui_egui::{Services, SlideApp};
use serde_json::json;

use super::fake_server::{Server, USER};
use super::*;

/// A cache folder of its own, removed afterwards.
struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Nextcloud signed in to the fake server (the password in memory: no keychain in tests).
fn signed_in(server: &Server, name: &str) -> (Nextcloud, Dir) {
    let dir = std::env::temp_dir().join(format!("slidelogic-nextcloud-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let nc = Nextcloud::new(None, Some(dir.clone()), |_, _| {});
    {
        let mut s = lock(&nc.state);
        s.stored.account = Some(Account { server: server.url.clone(), login: USER.into(), user: USER.into() });
        s.password = Some("app-password".into());
    }
    (nc, Dir(dir))
}

/// A one-slide presentation whose notes read `notes`.
fn deck(notes: &str) -> deckcraft_engine::model::Presentation {
    let mut s = Session::with_new();
    s.execute("slide.notes", &json!({"index": 0, "text": notes})).unwrap();
    (*s.active().unwrap().doc).clone()
}

fn pptx(notes: &str) -> Vec<u8> {
    deckcraft_engine::cmd::file::save_bytes(&deck(notes), "pptx").unwrap()
}

/// The first slide's notes of a .pptx.
fn notes_of(bytes: &[u8]) -> String {
    let mut s = Session::new();
    s.execute("file.openBytes", &json!({"name": "x.pptx", "data": deckcraft_engine::cmd::base64_encode(bytes)})).unwrap();
    s.active().unwrap().doc.slides[0].notes.text()
}

/// The presentation showing's first notes.
fn notes(app: &SlideApp) -> String {
    app.session.active().and_then(|d| d.doc.slides.first().map(|s| s.notes.text())).unwrap_or_default()
}

fn app(nc: &Nextcloud) -> SlideApp {
    SlideApp::new(Session::new(), Services { cloud: Some(nc.cloud()), ..Default::default() })
}

/// Runs the UI thread's side (the host, the background open, then the File view's answers)
/// until `done`, or fails after a while.
fn until(nc: &mut Nextcloud, app: &mut SlideApp, ctx: &egui::Context, what: &str, mut done: impl FnMut(&mut SlideApp) -> bool) {
    let start = Instant::now();
    loop {
        nc.poll(app, ctx);
        app.poll_opening();
        deckcraft_ui_egui::cloud_files::poll(app);
        if done(app) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn files_list_open_and_save_back_with_etags() {
    let server = Server::start();
    server.data().put("Talks/Aurora.pptx", pptx("Hello"));
    server.data().put("Talks/Budget.xlsx", b"not ours".to_vec());
    server.data().put("Talks/Old/Plan.deckcraft", b"native".to_vec());
    let (nc, _dir) = signed_in(&server, "files");
    let listing = run(&nc.state, "files.list", &json!({"path": "Talks"}), &|_| {}).unwrap();
    let names: Vec<&str> = listing["items"].as_array().unwrap().iter().filter_map(|i| i["name"].as_str()).collect();
    assert_eq!(names, ["Old", "Aurora.pptx"]);

    let opened = run(&nc.state, "files.open", &json!({"path": "Talks/Aurora.pptx", "openPaths": []}), &|_| {}).unwrap();
    let local = opened["localPath"].as_str().unwrap().to_string();
    assert_eq!(notes_of(&std::fs::read(&local).unwrap()), "Hello");
    assert_eq!((nc.cloud().location_of(&local)).as_deref(), Some("Talks/Aurora.pptx"));
    assert_eq!(opened["shared"], true, "a .pptx joins its co-editing session");
    let builds = server.data().session_of("Talks/Aurora.pptx").unwrap().builds.clone();
    assert_eq!(builds, [PROTOCOL.to_string()], "joined saying what its ops mean");
    let again = run(&nc.state, "files.open", &json!({"path": "Talks/Aurora.pptx", "openPaths": [local]}), &|_| {}).unwrap();
    assert_eq!(again["alreadyOpen"], true);
    assert!(run(&nc.state, "files.open", &json!({"path": "../Elsewhere/x.pptx", "openPaths": []}), &|_| {}).is_err());

    // Saved here: sent over the version it came from.
    std::fs::write(&local, pptx("Hello again")).unwrap();
    let remote = lock(&nc.state).stored.files.get(&local).cloned().unwrap();
    send_file(&nc.state, &local, &remote);
    assert_eq!(notes_of(&server.data().files["Talks/Aurora.pptx"].bytes), "Hello again");
    let etag = lock(&nc.state).stored.files[&local].etag.clone();
    assert_eq!(etag, Some(format!("\"{}\"", server.data().files["Talks/Aurora.pptx"].etag)), "the new version is remembered");

    // Someone else saved it meanwhile: a conflict, not an overwrite.
    server.data().put("Talks/Aurora.pptx", pptx("Their edit"));
    std::fs::write(&local, pptx("My edit")).unwrap();
    let remote = lock(&nc.state).stored.files.get(&local).cloned().unwrap();
    send_file(&nc.state, &local, &remote);
    assert_eq!(notes_of(&server.data().files["Talks/Aurora.pptx"].bytes), "Their edit");
    assert_eq!(lock(&nc.state).conflicts, [(local.clone(), "Aurora.pptx".to_string())]);
    // Keep mine.
    resolve(&nc.state, &local, "mine").unwrap();
    assert_eq!(notes_of(&server.data().files["Talks/Aurora.pptx"].bytes), "My edit");
    assert!(lock(&nc.state).conflicted.is_empty());
}

#[test]
fn save_as_asks_before_replacing_and_names_a_presentation() {
    let server = Server::start();
    server.data().put("Talks/Aurora.pptx", pptx("Hello"));
    let (nc, _dir) = signed_in(&server, "saveas");
    let there = save_as_target(&nc.state, "Talks", "Aurora", false).unwrap();
    assert_eq!(there, json!({"exists": true, "name": "Aurora.pptx"}));
    let new = save_as_target(&nc.state, "Talks", "Draft.deckcraft", false).unwrap();
    assert_eq!(new["name"], "Draft.deckcraft", "a presentation format stays");
    let notes = save_as_target(&nc.state, "Talks", "Notes.txt", false).unwrap();
    assert_eq!(notes["name"], "Notes.txt.pptx", "Save As writes a whole presentation");
    assert!(save_as_target(&nc.state, "../x", "Aurora", false).is_err());
    assert!(save_as_target(&nc.state, "Talks", "a/b", false).is_err());
}

#[test]
fn a_presentation_from_nextcloud_without_the_app_opens_and_saves_back() {
    let server = Server::start();
    server.data().put("Talks/Notes.pptx", pptx("First line"));
    // No SlideLogic app on this server: joining fails, and the presentation opens unshared.
    server.data().no_app = true;
    let (mut nc, _dir) = signed_in(&server, "unshared");
    let ctx = egui::Context::default();
    let mut app = app(&nc);
    deckcraft_ui_egui::cloud_files::open_remote(&mut app, "Talks/Gone.pptx");
    until(&mut nc, &mut app, &ctx, "an error for a missing file", |app| app.cloud_files.status.as_ref().is_some_and(|s| s.1));
    deckcraft_ui_egui::cloud_files::open_remote(&mut app, "Talks/Notes.pptx");
    until(&mut nc, &mut app, &ctx, "the presentation to open", |app| notes(app) == "First line");
    assert!(!app.session.collab_active());
    app.run("slide.notes", json!({"index": 0, "text": "First line and more"})).unwrap();
    nc.poll(&mut app, &ctx);
    // Save: the file goes to Nextcloud, over the version it came from.
    app.run("file.save", json!({})).unwrap();
    until(&mut nc, &mut app, &ctx, "the save in Nextcloud", |_| notes_of(&server.data().files["Talks/Notes.pptx"].bytes) == "First line and more");
    // Someone else changes it; the next save here is a conflict for the user to settle.
    server.data().put("Talks/Notes.pptx", pptx("Theirs"));
    app.run("slide.notes", json!({"index": 0, "text": "Mine!"})).unwrap();
    nc.poll(&mut app, &ctx);
    app.run("file.save", json!({})).unwrap();
    until(&mut nc, &mut app, &ctx, "the conflict", |app| !app.cloud_files.conflicts.is_empty());
    assert_eq!(notes_of(&server.data().files["Talks/Notes.pptx"].bytes), "Theirs", "not overwritten");
}

/// The browser's SlideLogic: an engine session sharing the same presentation through the log.
struct Browser {
    s: Session,
}

impl Browser {
    fn new(doc: deckcraft_engine::model::Presentation) -> Browser {
        let mut s = Session::new();
        s.add_document(DocState::new(doc, None, "Shared".into()));
        Browser { s }
    }

    /// Sends its edit to the server's log and takes the server's answer back.
    fn push(&mut self, server: &Server, path: &str) {
        let out = self.s.collab_outgoing().unwrap();
        let op = serde_json::to_string(&out.op).unwrap();
        let seq = {
            let mut d = server.data();
            let session = d.session_of(path).unwrap();
            let seq = session.log.last().map_or(session.checkpoint_seq, |e| e.0) + 1;
            session.log.push((seq, out.client, out.client_seq, out.base, op));
            seq
        };
        self.s.collab_receive(Incoming { seq, client: out.client, client_seq: out.client_seq, base: out.base, op: out.op }).unwrap();
    }

    /// Takes the log entries it hasn't had.
    fn catch_up(&mut self, server: &Server, path: &str) {
        let entries = server.data().session_of(path).unwrap().log.clone();
        for (seq, client, client_seq, base, op) in entries {
            if seq > self.s.collab_seq().unwrap() {
                let op = serde_json::from_str(&op).unwrap();
                self.s.collab_receive(Incoming { seq, client, client_seq, base, op }).unwrap();
            }
        }
    }

    fn notes(&self) -> String {
        self.s.active().unwrap().doc.slides[0].notes.text()
    }

    /// Types `text` at the end (or the start) of the first slide's notes, as a text command does.
    fn type_notes(&mut self, text: &str, at_start: bool) {
        let now = self.notes();
        let new = if at_start { format!("{text}{now}") } else { format!("{now}{text}") };
        self.s.execute("slide.notes", &json!({"index": 0, "text": new})).unwrap();
    }
}

#[test]
fn co_editing_with_the_browser_from_a_state_snapshot() {
    let path = "Shared/Plan.pptx";
    let server = Server::start();
    // The browser opened the file and started the session; it typed, and saved a checkpoint with
    // the engine's snapshot.
    let base = deck("Plan");
    let etag0 = server.data().put(path, deckcraft_engine::cmd::file::save_bytes(&base, "pptx").unwrap());
    let file_id = server.data().files[path].id;
    server.data().sessions.insert(
        file_id,
        super::fake_server::Session {
            file_id,
            checkpoint_etag: etag0,
            next_client: 1,
            members: vec![(1, "Bo Lee".into(), "{}".into(), true, true)],
            ..Default::default()
        },
    );
    let mut browser = Browser::new(base);
    browser.s.collab_start(1, 0).unwrap();
    browser.type_notes(" for the year", false);
    browser.push(&server, path);
    let (seq, state) = browser.s.collab_snapshot().unwrap().unwrap();
    // The file as the checkpoint wrote it. Its notes aren't the shared presentation's, so the
    // test tells the snapshot from the file.
    let etag = server.data().put(path, pptx("What the file says"));
    {
        let mut d = server.data();
        let session = d.session_of(path).unwrap();
        session.checkpoint_seq = seq;
        session.checkpoint_etag = etag;
        session.snapshot = Some((seq, state));
    }
    // More typing after the checkpoint.
    browser.type_notes("!", false);
    browser.push(&server, path);

    // The desktop opens it from Nextcloud and joins.
    let (mut nc, _dir) = signed_in(&server, "coedit");
    let ctx = egui::Context::default();
    let mut app = app(&nc);
    deckcraft_ui_egui::cloud_files::open_remote(&mut app, path);
    until(&mut nc, &mut app, &ctx, "the shared presentation", |app| app.session.collab_active());
    assert_eq!(notes(&app), browser.notes(), "the snapshot, then the log");
    assert_eq!(notes(&app), "Plan for the year!");
    let uid = app.session.active().unwrap().uid;
    assert!(app.session.collab_shares(uid));

    // The desktop types; the browser gets it.
    app.run("slide.notes", json!({"index": 0, "text": "Plan for the year! Signed off."})).unwrap();
    let desktop = app.session.collab_client().unwrap();
    until(&mut nc, &mut app, &ctx, "the desktop's edit in the log", |_| server.data().session_of(path).unwrap().log.iter().any(|e| e.1 == desktop));
    browser.catch_up(&server, path);
    assert_eq!(browser.notes(), "Plan for the year! Signed off.");

    // The browser types at the start while the desktop types at the end: both keep their typing.
    browser.type_notes("2027 ", true);
    browser.push(&server, path);
    app.run("slide.notes", json!({"index": 0, "text": "Plan for the year! Signed off. Thanks"})).unwrap();
    until(&mut nc, &mut app, &ctx, "both edits on the desktop", |app| notes(app) == "2027 Plan for the year! Signed off. Thanks");
    until(&mut nc, &mut app, &ctx, "the last edit confirmed", |app| !app.session.collab_pending());
    browser.catch_up(&server, path);
    assert_eq!(browser.notes(), "2027 Plan for the year! Signed off. Thanks");

    // The desktop's place reaches the others.
    until(&mut nc, &mut app, &ctx, "the desktop's place", |_| {
        server.data().session_of(path).unwrap().members.iter().any(|m| m.0 == desktop && m.2.contains("slide"))
    });

    // The browser's place reaches the desktop: drawn in the page's colour for it, with its name.
    let slide = browser.s.active().unwrap().doc.slides[0].id;
    let at = json!({"slide": slide.0, "shapes": []}).to_string();
    server.data().session_of(path).unwrap().members.iter_mut().filter(|m| m.0 == 1).for_each(|m| m.2 = at.clone());
    until(&mut nc, &mut app, &ctx, "the browser's place on the desktop", |app| {
        app.session.collab_others().first().is_some_and(|o| o.slide.is_some())
    });
    let other = app.session.collab_others()[0].clone();
    assert_eq!((other.client, other.name.as_str(), other.color), (1, "Bo Lee", [0xC0, 0x39, 0x2B]));
    assert_eq!(other.slide, Some(slide));

    // Save writes a checkpoint: the file, and the snapshot of the shared presentation beside it;
    // once it's in, the presentation counts as saved.
    let typed = app.session.collab_seq().unwrap();
    app.run("file.save", json!({})).unwrap();
    until(&mut nc, &mut app, &ctx, "the checkpoint", |_| !server.data().session_of(path).unwrap().checkpoints.is_empty());
    let (by, at, _, state) = server.data().session_of(path).unwrap().checkpoints[0].clone();
    assert_eq!(by, desktop);
    assert!(at >= typed, "the checkpoint has everything typed ({at} < {typed})");
    assert_eq!(notes_of(&server.data().files[path].bytes), "2027 Plan for the year! Signed off. Thanks");
    let mut joiner = Session::with_new();
    joiner.collab_load_snapshot(&state).unwrap();
    assert_eq!(joiner.active().unwrap().doc.slides[0].notes.text(), "2027 Plan for the year! Signed off. Thanks");
    until(&mut nc, &mut app, &ctx, "the presentation saved", |app| !app.session.active().unwrap().is_dirty());
    // The copy here follows the session's saves: opening it again later fetches Nextcloud's.
    let local = app.session.active().unwrap().path.clone().unwrap();
    let state = nc.state.clone();
    until(&mut nc, &mut app, &ctx, "the copy's record to follow the checkpoint", |_| {
        let saved = format!("\"{}\"", server.data().files[path].etag);
        lock(&state).stored.files.get(&local).is_some_and(|r| r.etag.as_deref() == Some(saved.as_str()))
    });

    // Quitting leaves the session.
    nc.shutdown(&mut app);
    assert!(!app.session.collab_active());
    let start = Instant::now();
    while server.data().session_of(path).unwrap().members.iter().any(|m| m.0 == desktop && m.4) {
        assert!(start.elapsed() < Duration::from_secs(5), "left the session");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(server.data().without_ocs_header.is_empty(), "every app call passes Nextcloud's CSRF check");
}

#[test]
fn joining_from_the_file_when_no_snapshot_is_kept() {
    let path = "Plain.pptx";
    let server = Server::start();
    server.data().put(path, pptx("Just the file"));
    let (mut nc, _dir) = signed_in(&server, "nosnapshot");
    let ctx = egui::Context::default();
    let mut app = app(&nc);
    deckcraft_ui_egui::cloud_files::open_remote(&mut app, path);
    until(&mut nc, &mut app, &ctx, "the shared presentation", |app| app.session.collab_active());
    assert_eq!(notes(&app), "Just the file");
    assert!(server.data().requests.iter().all(|(_, p)| !p.contains("collab/state")), "nothing to fetch");
    nc.shutdown(&mut app);
}

#[test]
fn one_presentation_is_shared_and_closing_it_leaves_the_session() {
    let server = Server::start();
    server.data().put("One.pptx", pptx("One"));
    server.data().put("Two.pptx", pptx("Two"));
    let (mut nc, _dir) = signed_in(&server, "one");
    let ctx = egui::Context::default();
    let mut app = app(&nc);
    deckcraft_ui_egui::cloud_files::open_remote(&mut app, "One.pptx");
    until(&mut nc, &mut app, &ctx, "the shared presentation", |app| app.session.collab_active());
    let client = app.session.collab_client().unwrap();
    // A second one from Nextcloud opens beside it, unshared.
    deckcraft_ui_egui::cloud_files::open_remote(&mut app, "Two.pptx");
    until(&mut nc, &mut app, &ctx, "the second presentation", |app| notes(app) == "Two");
    assert!(server.data().session_of("Two.pptx").is_none(), "not joined");
    let one = app.session.documents().iter().position(|d| d.doc.slides[0].notes.text() == "One").unwrap();
    assert!(app.session.collab_shares(app.session.documents()[one].uid), "still the shared one");
    // Closing the shared one ends sharing.
    app.session.close_document(one);
    until(&mut nc, &mut app, &ctx, "sharing to end", |_| server.data().session_of("One.pptx").unwrap().members.iter().any(|m| m.0 == client && !m.4));
    assert!(!app.session.collab_active());
}

#[test]
fn a_lost_session_is_rejoined_from_the_file() {
    let path = "Rejoin.pptx";
    let server = Server::start();
    server.data().put(path, pptx("Before"));
    let (mut nc, _dir) = signed_in(&server, "rejoin");
    let ctx = egui::Context::default();
    let mut app = app(&nc);
    deckcraft_ui_egui::cloud_files::open_remote(&mut app, path);
    until(&mut nc, &mut app, &ctx, "the shared presentation", |app| app.session.collab_active());
    // The server forgets the session (cleaned up, or restarted): this copy can't follow it.
    server.data().sessions.clear();
    server.data().put(path, pptx("Changed meanwhile"));
    until(&mut nc, &mut app, &ctx, "a new session, from the file as it is now", |app| {
        app.session.collab_active() && notes(app) == "Changed meanwhile"
    });
    assert_eq!(server.data().sessions.len(), 1, "joined again");
    assert_eq!(app.session.documents().len(), 1, "in place of the old copy");
    nc.shutdown(&mut app);
}

#[test]
fn a_session_on_a_newer_slidelogic_opens_unshared_and_says_why() {
    let path = "Newer.pptx";
    let server = Server::start();
    server.data().put(path, pptx("Edited elsewhere"));
    let file_id = server.data().files[path].id;
    let etag = server.data().files[path].etag.clone();
    // Someone joined with a SlideLogic whose edits this one would misread.
    server.data().sessions.insert(
        file_id,
        super::fake_server::Session { file_id, checkpoint_etag: etag, next_client: 1, build: (PROTOCOL + 1).to_string(), ..Default::default() },
    );
    let (mut nc, _dir) = signed_in(&server, "newer");
    let ctx = egui::Context::default();
    let mut app = app(&nc);
    deckcraft_ui_egui::cloud_files::open_remote(&mut app, path);
    until(&mut nc, &mut app, &ctx, "the presentation, unshared", |app| notes(app) == "Edited elsewhere" && app.cloud_files.notice.is_some());
    assert!(!app.session.collab_active());
    let (title, text) = app.cloud_files.notice.clone().unwrap();
    assert_eq!((title.as_str(), text.as_str()), ("Opened without editing together", coedit::OUTDATED));
    let left = server.data().session_of(path).unwrap().members.iter().all(|m| !m.4);
    assert!(left, "it left the session it couldn't follow");
}

#[test]
fn quitting_saves_what_the_file_lacks_before_leaving() {
    let path = "Quit.pptx";
    let server = Server::start();
    server.data().put(path, pptx("Before"));
    let (mut nc, _dir) = signed_in(&server, "quit");
    let ctx = egui::Context::default();
    let mut app = app(&nc);
    deckcraft_ui_egui::cloud_files::open_remote(&mut app, path);
    until(&mut nc, &mut app, &ctx, "the shared presentation", |app| app.session.collab_active());
    app.run("slide.notes", json!({"index": 0, "text": "Typed just before quitting"})).unwrap();
    until(&mut nc, &mut app, &ctx, "the edit confirmed", |app| !app.session.collab_pending());
    assert!(server.data().session_of(path).unwrap().checkpoints.is_empty(), "nothing saved yet");
    // Quit at once: the edit is in the log, and goes to the file before the desktop leaves.
    nc.shutdown(&mut app);
    assert_eq!(notes_of(&server.data().files[path].bytes), "Typed just before quitting");
    let s = server.data().session_of(path).unwrap().checkpoints.clone();
    assert_eq!(s.len(), 1, "one checkpoint, with its snapshot");
    assert!(!s[0].3.is_empty());
}
