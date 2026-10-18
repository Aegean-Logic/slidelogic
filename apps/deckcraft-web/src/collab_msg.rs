//! Co-editing messages from the page (`?host=parent`), checked before they reach the session.
//! No browser types here, so it's tested natively. The protocol is in `packaging/web/README.md`
//! (§ Co-editing) and `../deckcraft-nextcloud` (README, § Co-editing).

use deckcraft_engine::collab::{Incoming, Op, Other};
use deckcraft_engine::model::{ShapeId, SlideId};
use serde_json::Value;

/// The largest log batch, state snapshot or presence list taken in one message (JSON text).
pub const MAX_JSON_BYTES: usize = 512 << 20;
/// The most people drawn.
const MAX_PEOPLE: usize = 64;
/// The largest integer a JavaScript number holds exactly.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// What the page asks of the shared session.
#[derive(Debug, PartialEq)]
pub enum FromPage {
    /// `deckcraft:collab:start`: the presentation just opened is the shared one as of log
    /// position `seq` (or, with `state`, the snapshot is: it replaces the file just opened); this
    /// person is `client`, and with `read_only` can't edit. `user` and `color`: how the others see
    /// them.
    Start { client: u64, seq: u64, read_only: bool, user: Option<String>, state: Option<String> },
    /// `deckcraft:collab:entries {json}`: log entries, in order.
    Entries(Vec<Incoming>),
    /// `deckcraft:collab:checkpoint {id}`: send the shared presentation to save.
    Checkpoint { id: u64 },
    /// `deckcraft:collab:stop`: the session ended.
    Stop,
    /// `deckcraft:collab:presence {json}`: where the others are.
    Presence(Vec<Other>),
}

/// The fields of a page message a co-editing message may carry (read from the browser's object).
#[derive(Debug, Default)]
pub struct Fields {
    pub client: Option<f64>,
    /// `clientId`, the same as `client` (either will do).
    pub client_id: Option<f64>,
    pub seq: Option<f64>,
    pub id: Option<f64>,
    pub read_only: Option<bool>,
    pub json: Option<String>,
    pub user: Option<String>,
    pub state: Option<String>,
}

/// The co-editing message `kind` with `f`, if it is one; `Err` for one that's malformed (told
/// back to the page).
pub fn parse(kind: &str, f: Fields) -> Option<Result<FromPage, String>> {
    let need = |v: Option<f64>, what: &str| whole(v).ok_or_else(|| format!("{kind}: `{what}` must be a whole number"));
    Some(match kind {
        "deckcraft:collab:start" => (|| {
            let client = need(f.client.or(f.client_id), "client")?;
            if client == 0 {
                return Err(format!("{kind}: `client` must not be 0"));
            }
            let seq = need(f.seq, "seq")?;
            let state = match f.state {
                Some(s) if s.len() > MAX_JSON_BYTES => return Err(format!("{kind}: `state` is too large")),
                Some(s) if s.trim().is_empty() => None,
                other => other,
            };
            let user = f.user.map(|u| name(&u)).filter(|u| !u.is_empty());
            Ok(FromPage::Start { client, seq, read_only: f.read_only.unwrap_or(false), user, state })
        })(),
        "deckcraft:collab:entries" => match f.json {
            None => Err(format!("{kind}: `json` is missing")),
            Some(text) if text.len() > MAX_JSON_BYTES => Err(format!("{kind}: more than {MAX_JSON_BYTES} bytes")),
            Some(text) => entries(&text).map(FromPage::Entries).map_err(|e| format!("{kind}: {e}")),
        },
        "deckcraft:collab:checkpoint" => need(f.id, "id").map(|id| FromPage::Checkpoint { id }),
        "deckcraft:collab:stop" => Ok(FromPage::Stop),
        "deckcraft:collab:presence" => match f.json {
            Some(text) if text.len() <= MAX_JSON_BYTES => people(&text).map(FromPage::Presence).map_err(|e| format!("{kind}: {e}")),
            _ => Err(format!("{kind}: `json` is missing or too large")),
        },
        _ => return None,
    })
}

/// Log entries from JSON text. An entry whose op doesn't parse (a bug or a tampered client
/// elsewhere) keeps its place in the log as a no-op: every copy reads it the same way, so they
/// stay together, where refusing the batch would stall them all.
fn entries(text: &str) -> Result<Vec<Incoming>, String> {
    let raw: Vec<Value> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    raw.into_iter()
        .map(|v| {
            let num = |key: &str| v.get(key).and_then(Value::as_u64).ok_or_else(|| format!("an entry without `{key}`"));
            let op: Op = v.get("op").cloned().and_then(|o| serde_json::from_value(o).ok()).unwrap_or_default();
            Ok(Incoming { seq: num("seq")?, client: num("client")?, client_seq: num("client_seq")?, base: num("base")?, op })
        })
        .collect()
}

/// The others from JSON text: `[{client, name, color, slide, shapes}]` (each person's
/// `Session::collab_presence` with their client id, name and colour). Someone whose place doesn't
/// read is left out rather than the whole list refused.
fn people(text: &str) -> Result<Vec<Other>, String> {
    let raw: Vec<Value> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let id = |v: &Value| v.as_u64().and_then(|n| u32::try_from(n).ok());
    Ok(raw
        .into_iter()
        .filter_map(|v| {
            let client = v.get("client").and_then(Value::as_u64)?;
            let slide = match v.get("slide") {
                None | Some(Value::Null) => None,
                Some(s) => Some(SlideId(id(s)?)),
            };
            let shapes = v.get("shapes").and_then(Value::as_array).map(|a| a.iter().filter_map(id).map(ShapeId).collect()).unwrap_or_default();
            let color = v.get("color").and_then(Value::as_str).and_then(rgb).unwrap_or([128, 128, 128]);
            Some(Other::new(client, v.get("name").and_then(Value::as_str).unwrap_or(""), color, slide, shapes))
        })
        .take(MAX_PEOPLE)
        .collect())
}

/// A name as shown: one line, at most 60 characters.
fn name(s: &str) -> String {
    s.trim().chars().filter(|c| !c.is_control()).take(60).collect()
}

/// `#rrggbb` as RGB.
fn rgb(s: &str) -> Option<[u8; 3]> {
    let hex = s.strip_prefix('#')?;
    let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    match (hex.len(), byte(0), byte(2), byte(4)) {
        (6, Some(r), Some(g), Some(b)) => Some([r, g, b]),
        _ => None,
    }
}

fn whole(v: Option<f64>) -> Option<u64> {
    let v = v?;
    ((0.0..=MAX_SAFE_INTEGER).contains(&v) && v.fract() == 0.0).then_some(v as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(f: Fields) -> Option<Result<FromPage, String>> {
        parse("deckcraft:collab:start", f)
    }

    #[test]
    fn start_is_checked() {
        assert_eq!(parse("deckcraft:open", Fields::default()), None, "not a co-editing message");
        assert_eq!(
            start(Fields { client: Some(3.0), seq: Some(0.0), ..Default::default() }),
            Some(Ok(FromPage::Start { client: 3, seq: 0, read_only: false, user: None, state: None }))
        );
        assert_eq!(
            start(Fields {
                client_id: Some(4.0),
                seq: Some(12.0),
                read_only: Some(true),
                user: Some("  Ada Lovelace\n".into()),
                state: Some("UEsDBA==".into()),
                ..Default::default()
            }),
            Some(Ok(FromPage::Start { client: 4, seq: 12, read_only: true, user: Some("Ada Lovelace".into()), state: Some("UEsDBA==".into()) }))
        );
        assert!(matches!(
            start(Fields { client: Some(3.0), seq: Some(0.0), state: Some(" ".into()), ..Default::default() }),
            Some(Ok(FromPage::Start { state: None, .. }))
        ));
        assert!(matches!(start(Fields { client: Some(3.5), seq: Some(0.0), ..Default::default() }), Some(Err(_))));
        assert!(matches!(start(Fields { client: Some(0.0), seq: Some(0.0), ..Default::default() }), Some(Err(_))), "0 isn't a client");
        assert!(matches!(start(Fields { client: Some(-1.0), seq: Some(0.0), ..Default::default() }), Some(Err(_))));
        assert!(matches!(start(Fields { client: Some(3.0), ..Default::default() }), Some(Err(_))), "no seq");
        assert_eq!(parse("deckcraft:collab:checkpoint", Fields { id: Some(7.0), ..Default::default() }), Some(Ok(FromPage::Checkpoint { id: 7 })));
        assert!(matches!(parse("deckcraft:collab:checkpoint", Fields::default()), Some(Err(_))));
        assert_eq!(parse("deckcraft:collab:stop", Fields::default()), Some(Ok(FromPage::Stop)));
    }

    #[test]
    fn log_entries_are_checked() {
        let entries = |json: &str| parse("deckcraft:collab:entries", Fields { json: Some(json.into()), ..Default::default() });
        let entry = Incoming { seq: 1, client: 2, client_seq: 1, base: 0, op: Op::default() };
        let json = serde_json::to_string(&vec![entry.clone()]).unwrap_or_default();
        assert_eq!(entries(&json), Some(Ok(FromPage::Entries(vec![entry]))));
        // As the page sends them: snake_case numbers, the op passed through untouched.
        let page = r#"[{"seq":2,"client":3,"client_seq":1,"base":1,"op":{"fields":{"first_slide_number":4}}}]"#;
        match entries(page) {
            Some(Ok(FromPage::Entries(e))) => assert!(e.len() == 1 && e[0].client == 3 && !e[0].op.is_empty()),
            other => panic!("{other:?}"),
        }
        assert!(matches!(entries("[{]"), Some(Err(_))));
        // A garbled op is a no-op in its place, not a stalled session.
        let garbled = r#"[{"seq":4,"client":2,"client_seq":9,"base":3,"op":{"slides":"nonsense"}}]"#;
        match entries(garbled) {
            Some(Ok(FromPage::Entries(e))) => assert!(e.len() == 1 && e[0].seq == 4 && e[0].op.is_empty()),
            other => panic!("{other:?}"),
        }
        assert!(matches!(entries(r#"[{"client":2}]"#), Some(Err(_))), "no seq");
        assert!(matches!(parse("deckcraft:collab:entries", Fields::default()), Some(Err(_))));
    }

    #[test]
    fn the_others_are_checked() {
        let presence = |json: &str| parse("deckcraft:collab:presence", Fields { json: Some(json.into()), ..Default::default() });
        let long = "x".repeat(500);
        let people = format!(
            r##"[{{"client":3,"name":"{long}","color":"#12718C","slide":257,"shapes":[2,3,"x",-1]}},
               {{"client":4,"name":"Ben","color":"blue","slide":null,"shapes":[]}},
               {{"name":"No client"}},
               {{"client":6,"slide":99999999999}}]"##
        );
        match presence(&people) {
            Some(Ok(FromPage::Presence(p))) => {
                assert_eq!(p.len(), 2, "without a client, or on a slide that can't be, someone is left out");
                assert_eq!(p[0].name.chars().count(), 60);
                assert_eq!(p[0].color, [0x12, 0x71, 0x8C]);
                assert_eq!(p[0].slide, Some(SlideId(257)));
                assert_eq!(p[0].shapes, vec![ShapeId(2), ShapeId(3)]);
                assert_eq!(p[1].slide, None, "in a master view");
                assert_eq!(p[1].color, [128, 128, 128], "not #rrggbb: grey");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(presence("{}"), Some(Err(_))));
        let many = format!("[{}]", vec![r#"{"client":1,"slide":1}"#; 500].join(","));
        assert!(matches!(presence(&many), Some(Ok(FromPage::Presence(p))) if p.len() == MAX_PEOPLE));
    }
}
