//! The File view's Nextcloud places through real egui input, against a cloud that answers at once
//! (the desktop app's own requests are tested against a fake Nextcloud in `apps/deckcraft`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use deckcraft_engine::Session;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value as Json, json};

use crate::backstage::Page;
use crate::cloud::{Cloud, CloudInbox};
use crate::{Services, SlideApp};

/// A cloud signed in as ana, with `Talks/` and `Aurora.pptx` at the top; opening a file answers
/// with `local`. Every operation asked is recorded.
fn cloud(local: String) -> (Cloud, Arc<Mutex<Vec<(String, Json)>>>) {
    let inbox = CloudInbox::default();
    let asked: Arc<Mutex<Vec<(String, Json)>>> = Arc::default();
    let (answers, log) = (inbox.clone(), asked.clone());
    let request = move |id: u64, op: &str, params: Json| {
        log.lock().unwrap().push((op.to_string(), params.clone()));
        let answer = match op {
            "account.info" => json!({"signedIn": true, "account": "ana on cloud.example", "server": "https://cloud.example"}),
            "files.list" if params["path"] == "" => json!({"path": "", "items": [
                {"name": "Talks", "path": "Talks", "folder": true},
                {"name": "Aurora.pptx", "path": "Aurora.pptx", "folder": false},
            ]}),
            "files.list" => json!({"path": params["path"], "items": []}),
            "files.open" => json!({"localPath": local}),
            _ => json!({}),
        };
        answers.lock().unwrap().push((id, Ok(answer)));
    };
    (Cloud { request: Box::new(request), inbox, files: true, location: None }, asked)
}

fn harness(app: SlideApp) -> Harness<'static, SlideApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

#[test]
fn account_open_and_save_as_show_nextcloud() {
    let dir = std::env::temp_dir().join(format!("slidelogic-cloud-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let local = dir.join("Aurora.pptx").to_string_lossy().to_string();
    let mut s = Session::with_new();
    s.execute("slide.notes", &json!({"index": 0, "text": "From Nextcloud"})).unwrap();
    std::fs::write(&local, deckcraft_engine::cmd::file::save_bytes(&s.active().unwrap().doc, "pptx").unwrap()).unwrap();
    let (cloud, asked) = cloud(local.clone());
    let mut h = harness(SlideApp::new(Session::with_new(), Services { cloud: Some(cloud), ..Default::default() }));

    // File › Account: who's signed in, and Sign out.
    h.state_mut().open_backstage(Page::Account);
    h.run_steps(3);
    h.get_by_label_contains("Signed in as ana on cloud.example");
    h.get_by_role_and_label(Role::Button, "Sign out");
    if let Ok(img) = h.render() {
        let _ = img.save(dir.join("account.png"));
    }

    // File › Open › Nextcloud: its folders and presentations; one picked opens.
    h.state_mut().open_backstage(Page::Open);
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Nextcloud").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Talks");
    if let Ok(img) = h.render() {
        let _ = img.save(dir.join("open.png"));
    }
    h.get_by_role_and_label(Role::Button, "Aurora.pptx").click();
    for _ in 0..200 {
        h.run_steps(1);
        if h.state().session.documents().len() == 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let app = h.state();
    assert_eq!(app.session.documents().len(), 2, "opened beside the first");
    let d = app.session.active().unwrap();
    assert_eq!((d.path.as_deref(), d.doc.slides[0].notes.text().as_str()), (Some(local.as_str()), "From Nextcloud"));
    assert!(app.backstage.is_none(), "the File view closed on the presentation");
    let open = asked.lock().unwrap().iter().find(|(op, _)| op == "files.open").cloned().unwrap();
    assert_eq!(open.1["path"], "Aurora.pptx");

    // File › Save As › Nextcloud: the presentation's name to start with.
    h.state_mut().open_backstage(Page::SaveAs);
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Save to Nextcloud");
    assert_eq!(h.state().cloud_files.name, "Aurora");
    assert!(h.state().cloud_files.status.is_none(), "what opening said went to the status bar");
    if let Ok(img) = h.render() {
        let _ = img.save(dir.join("save-as.png"));
    }
}

#[test]
fn without_a_cloud_the_file_view_has_no_nextcloud() {
    let mut h = harness(SlideApp::new(Session::with_new(), Services::default()));
    h.state_mut().open_backstage(Page::Open);
    h.run_steps(3);
    assert!(h.query_by_role_and_label(Role::Button, "Nextcloud").is_none());
    h.state_mut().open_backstage(Page::Account);
    h.run_steps(3);
    h.get_by_label_contains("SlideLogic has no accounts");
}
