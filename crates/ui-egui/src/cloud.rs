//! The host's cloud (Nextcloud), as OneDrive is PowerPoint's: the File view's Nextcloud places
//! ([`crate::cloud_files`]) talk to it through [`Cloud`]. The host does the talking (the desktop
//! app over HTTP); the UI only sends requests and reads the answers. Ported from WordLogic's
//! `crates/ui-egui/src/cloud.rs` (GridLogic's definitions), without the Share dialog's operations.

use std::sync::{Arc, Mutex};

use serde_json::Value as Json;

/// Answers from the cloud: (request id, result).
pub type CloudInbox = Arc<Mutex<Vec<(u64, Result<Json, String>)>>>;

/// The host's cloud. `request(id, op, params)` starts an operation and returns at once; its
/// answer arrives in `inbox` under the same id (the host asks for a repaint):
///
/// - `account.info` `{}` → `{signedIn, account?, server?}`; `account.signOut` `{}` → the same
/// - `account.signIn` `{server}` → the same: signs in to the cloud at that address
/// - `files.list` `{path}` → `{path, items: [{name, path, folder, size}]}`
/// - `files.open` `{path, openPaths}` → `{localPath, alreadyOpen?}`: a copy here, tied to the
///   cloud file (open it like any file; saving it saves to the cloud)
/// - `files.saveAs` `{folder, name, replace}` → `{localPath, name}` (save the presentation there;
///   it goes to the cloud), or `{exists: true, name}`
/// - `files.resolve` `{localPath, choice}` → what to do after a sync conflict
///   ([`crate::cloud_files::Conflict`]): `choice` `keepBoth` → `{localPath, saveAs: true, name}`,
///   `mine` → `{name}`, `theirs` → `{localPath, reload: true}`
pub struct Cloud {
    pub request: Box<dyn Fn(u64, &str, Json)>,
    pub inbox: CloudInbox,
    /// The File view can open and save cloud files (account, Open, Save As).
    pub files: bool,
    /// The cloud file a local file is the copy of (`folder/name.pptx`), if it is one.
    pub location: Option<Box<dyn Fn(&str) -> Option<String>>>,
}

impl Cloud {
    /// The cloud file `local` is the copy of, if any.
    pub fn location_of(&self, local: &str) -> Option<String> {
        self.location.as_ref().and_then(|f| f(local))
    }
}

/// A request id no other cloud request in this app uses (answers share one inbox).
pub fn next_request_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
