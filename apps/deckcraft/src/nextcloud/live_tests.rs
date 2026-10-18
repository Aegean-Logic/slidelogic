//! Co-editing against a real Nextcloud with the SlideLogic app (not run by default): the desktop's
//! side of a session with someone in the browser. Signed in with an app password from the
//! environment (no keychain, no Login Flow):
//!
//! ```text
//! DECKCRAFT_LIVE_SERVER=http://localhost:8080 DECKCRAFT_LIVE_USER=… DECKCRAFT_LIVE_PASSWORD=… \
//! DECKCRAFT_LIVE_FILE="SlideLogic/SlideLogic test.pptx" \
//! cargo test -p deckcraft live -- --ignored --nocapture
//! ```
//!
//! The desktop opens the file, joins, and adds ` [desktop]` to the first slide's notes; then it
//! waits up to two minutes for `[browser]` typed in those notes in the browser, and quits (which
//! saves a checkpoint, then leaves).

use std::time::{Duration, Instant};

use deckcraft_engine::Session;
use deckcraft_ui_egui::{Services, SlideApp};
use serde_json::json;

use super::*;

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("{k} isn't set"))
}

fn notes(app: &SlideApp) -> String {
    app.session.active().and_then(|d| d.doc.slides.first().map(|s| s.notes.text())).unwrap_or_default()
}

fn until(nc: &mut Nextcloud, app: &mut SlideApp, ctx: &egui::Context, what: &str, secs: u64, mut done: impl FnMut(&mut SlideApp) -> bool) {
    let start = Instant::now();
    eprintln!("live: waiting for {what}");
    loop {
        nc.poll(app, ctx);
        app.poll_opening();
        deckcraft_ui_egui::cloud_files::poll(app);
        if let Some((title, text)) = app.cloud_files.notice.take() {
            eprintln!("live: notice: {title}: {text}");
        }
        if done(app) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(secs), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "needs a Nextcloud with the SlideLogic app; see the module's documentation"]
fn live_co_editing_with_the_browser() {
    let dir = std::env::temp_dir().join(format!("slidelogic-live-{}", std::process::id()));
    let mut nc = Nextcloud::new(None, Some(dir.clone()), |_, _| {});
    {
        let mut s = lock(&nc.state);
        let user = env("DECKCRAFT_LIVE_USER");
        s.stored.account = Some(Account { server: normalise_server(&env("DECKCRAFT_LIVE_SERVER")).unwrap(), login: user.clone(), user });
        s.password = Some(env("DECKCRAFT_LIVE_PASSWORD"));
    }
    let file = env("DECKCRAFT_LIVE_FILE");
    let ctx = egui::Context::default();
    let mut app = SlideApp::new(Session::new(), Services { cloud: Some(nc.cloud()), ..Default::default() });
    deckcraft_ui_egui::cloud_files::open_remote(&mut app, &file);
    until(&mut nc, &mut app, &ctx, "the shared presentation", 60, |app| app.session.collab_active());
    eprintln!("live: joined as client {:?} at {:?}; notes: {:?}", app.session.collab_client(), app.session.collab_seq(), notes(&app));
    let typed = format!("{} [desktop]", notes(&app));
    app.run("slide.notes", json!({"index": 0, "text": typed})).unwrap();
    until(&mut nc, &mut app, &ctx, "the desktop's edit confirmed", 30, |app| !app.session.collab_pending());
    eprintln!("live: sent; now type [browser] in the first slide's notes in the browser");
    until(&mut nc, &mut app, &ctx, "[browser] from the browser", 120, |app| notes(app).contains("[browser]"));
    eprintln!("live: notes now {:?}; others: {:?}", notes(&app), app.session.collab_others());
    assert!(notes(&app).contains("[desktop]"), "the desktop's typing kept");
    // Quitting saves what the file lacks, then leaves.
    nc.shutdown(&mut app);
    eprintln!("live: left at {:?}", app.session.collab_seq());
    let _ = std::fs::remove_dir_all(&dir);
}
