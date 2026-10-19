# Real-time co-editing: design

> **Status (2026-10-18):** phases 1–3 done. Everyone with a presentation open from Nextcloud
> (`../deckcraft-nextcloud` 0.2.0) edits it together, live, in the browser. Fork-specific (Aegean
> Office), not upstream behaviour. GridLogic (`../gridcraft/docs/co-editing.md`) and WordLogic
> (`../wordcraft/docs/co-editing.md`) work the same way; SlideLogic follows them.

## Where it stands

- **Edits:** every edit, from the ribbon, keys, pointer gestures or agents, shows in the others'
  copies within about a second: new, deleted and moved slides, shapes, formatting, text,
  transitions and animations, design changes.
- **Typing together:** several people can type in the same text box, table cell or speaker
  notes at once (in grouped shapes too) and all keep what they typed; one bolding words while another colours them keeps both; someone moving or
  restyling a box while another types in it keeps both. Each person's caret stays with their
  own text, and each stays on their slide when others add or remove slides in front of it.
  Speaker notes are typed into a draft that's saved (one undo step) when the field loses focus:
  the others see that typing once it's saved, and meanwhile the draft takes in what they type
  in those notes, so saving it keeps both.
- **Presence:** the shapes each person has selected are outlined on the slide in their colour
  with their name; their initials show on the thumbnail of the slide they're on; a strip above
  the editor lists who's editing.
- **Undo and Redo** take back a person's own edits only, never someone else's later change.
- **View-only members** (they can read the file but not write it) follow the others live; an
  edit they try is refused with a message, and the server refuses it too.
- **Saving:** the member with the lowest client id saves the shared presentation to the file every
  20 edits or 30 seconds, and on Save, with a state snapshot for people joining later. A file
  changed outside the session (another app, a sync client) ends it with a message.
- **Shared links** (`/s/{token}`, Nextcloud app 0.3.0): a link to a presentation opens it in
  SlideLogic. Edit links co-edit as guests under the name they gave Nextcloud; view-only links
  follow live. Presentations in a shared folder's link open the same way.
- **The desktop app** edits together with the browser: File › Account signs in to Nextcloud
  (Login Flow v2 in the browser; the app password goes to the system keychain), File › Open ›
  Nextcloud browses its folders, and a presentation opened from there joins its session
  (`apps/deckcraft/src/nextcloud.rs`, `nextcloud/coedit.rs`, ported from WordLogic). It works
  on a copy in the cache folder tied to the Nextcloud file: Save As › Nextcloud and Recent go
  through Nextcloud too, and a presentation not being co-edited is sent back on Save over the
  version it came from (someone else's save in between asks which to keep). One presentation is
  shared at a time; another opened from Nextcloud meanwhile is sent back on Save as above. A
  checkpoint that holds exactly what the desktop shows marks it saved.

Checked end to end in the test Nextcloud (2026-10-18): two tabs on one .pptx, then a third
joining late from the checkpoint snapshot. New slides made in each tab, and at once in both,
arrived in the others; all three copies ended with the same presentation (the same
`presentation.json` but for each copy's own id counter). Undo in one tab took back only its own
slide; the file was saved at the checkpoint with its snapshot; another tab's selection showed
outlined with its name; a write to the file from outside ended the session with the message.
Then shared links: gaz in the editor, a guest on an edit link and a guest on a view-only link to
the same file, and later a guest opening it from a link to its folder, all in one session. The
guests' new slides (file link and folder link) reached everyone, and the view-only guest
followed live; their New Slide was refused with "You can view this presentation but not edit
it." and nothing was sent. Paths out of the shared folder, missing files and unknown tokens
answered 404.

And a view-only member with their own account: a test user given the file through a read-only
user share, signed in beside gaz (on `127.0.0.1:8080`, so both sessions held). They joined the
same session as "(viewing)", with "You can view this presentation but not edit it." over the
editor; gaz saw them in the strip and on the thumbnails; gaz's new slide reached them; their New
Slide was refused with the same message in the status bar and nothing was sent; a push or a
checkpoint made directly with their session was refused by the server (403). The status bar
used to draw a long message under its own buttons in a narrow window; it now cuts the message
short with "…" before them, the whole message on hover.

Found on the way: a tab loaded in the background crashed SlideLogic when the session's first
messages arrived before its first frame (egui has no fonts until then). The bridge now holds
co-editing messages until the first frame; such a tab joins when it's first shown.

Phase 2–3 pieces:

| Piece | Code | Proof |
|---|---|---|
| Session hook | `collab/shared.rs`: `Session::collab_start`, `collab_outgoing`, `collab_receive`, `collab_presence`, `collab_set_others`, `collab_snapshot`, `collab_load_snapshot`, `collab_set_read_only` | After every outermost command, and whenever the host asks for the next op, the presentation is compared with the one last shared and the difference recorded as this person's edit, so nothing has to call in. A new undo step starts when the session's own history got one (so steps match SlideLogic's undo without co-editing). Mid-gesture (a drag) or mid-composition nothing goes out; an op arriving then is applied to the gesture's base too, and the gesture goes out whole, as one step. The fuzz runs through this hook (150 seeds × 1,500 steps pass); tests for view-only members, typing, presence, a drag interrupted by a remote op, a late joiner from a snapshot |
| Undo | `cmd::edit` `undo`/`redo` and `can_undo`/`can_redo` defer to `Session::collab_undo` for the shared presentation | Snapshot undo would take back everyone's edits since |
| State snapshots | `Session::collab_snapshot` (the native format, base64) | A .pptx read back numbers slides afresh, so joiners start from the snapshot: `a_late_joiner_starts_from_the_checkpoint_snapshot` |
| Presence | `ui-egui/presence.rs` (canvas outlines and name tags, thumbnail initials) | `presence::tests` |
| Bridge | `apps/deckcraft-web/src/host.rs`, `collab_msg.rs`; protocol in `packaging/web/README.md`, Co-editing | Messages checked natively (`collab_msg` tests); handled as they arrive, so a background tab keeps up and answers checkpoints |
| Sequencer | `../deckcraft-nextcloud`: `CollabController`, `CollabService`, `SnapshotStore`, migration, `CleanUpSessions`, `src/collab.js` | WordLogic's, ported; end to end above |

Phase 1 pieces:

| Piece | Code | Proof |
|---|---|---|
| Ops from results | `collab::capture(before, after)`, `collab::apply` | Every `Presentation` and `Slide` field is shared or named as handled elsewhere: the lists destructure the structs, so a new model field doesn't compile until it's placed. Capture then apply rebuilds every edit of the fuzz exactly |
| Slide, master and shape order | `collab::SeqEdit`, `seq::diff` / `apply` / `rebase` | Every pair of single edits (insert, remove, move) of a 4-item list commutes (exhaustive unit test) |
| Collision-free ids | `collab::id_space`, `claim_ids` | Each client takes new ids from a range of its own; `Replica::local` refuses an edit whose ids left it |
| Replica | `collab::Replica` (`local`, `outgoing`, `receive`, `undo`) | `copies_converge_whatever_the_timing`: 2 and 3 people, random commands, ops through a fake server as JSON, random delivery; the copies end the same, valid, and save to the same .pptx. `copies_converge_long` (ignored): 300 seeds × 1,500 steps |
| Own-edit undo | `Replica::undo` | Undo then redo of a fresh edit gives back the exact presentation (fuzz); undo takes back only the person's own edit and skips what someone else changed since (scenario tests) |

Scenario tests (`crates/engine/src/collab/tests.rs`): two new slides at once both stay; moves of
different shapes on one slide both stay; the same shape moved twice takes the later move; a slide
deleted while someone edits it stays deleted; two slide moves both stay; two people grouping the
same shapes get one group; a late op is placed after what its author missed; hostile ops (too
many edits, unknown or mistyped settings, damaged media, edits naming slides that aren't there)
are refused or skipped, never crashed on.

Found and fixed on the way (single-user bugs too): pasted slides and slides made from an outline
joined no section (`cmd::slide::tidy_sections`, with regression tests).

## 1. What the engine gives us

- Every command runs through `Session::execute` and leaves the presentation as a new
  `Arc<Presentation>` (undo keeps the previous ones). The model is shared structure:
  `Presentation::slides` is `Vec<Arc<Slide>>`, masters are `Vec<Arc<Master>>`, and an edit
  replaces only the `Arc`s it touched. Comparing pointers finds what a command changed without
  looking inside the rest.
- Slides, layouts, masters, shapes and media have stable ids (`SlideId`, `LayoutId`, `MasterId`,
  `ShapeId`, `MediaId`), all from one counter per presentation (`Presentation::next_id`, through
  `alloc_id`). Shape ids only have to be unique on their slide.
- Selection and view state live in `DocState::selection`, outside the presentation, so they never
  travel.

## 2. Approach: send results, as GridLogic does

An op (`collab::Op`) is the **result** of a local command, not the command:

1. **Order**: slides, masters and each slide's shapes as `SeqEdit`s by id: `Insert { id, after }`,
   `Move { id, after, from }`, `Remove { id, after }`. Places are named by neighbour ("after slide
   7"), so someone else's insert can't shift them; when the neighbour was removed or moved, the
   rebase names the neighbour's own old neighbour. These are the only parts transformed against
   concurrent ones. `seq::diff` derives them from two orders, moving only the ids outside the
   longest run that kept its order.
2. **Content**: new slides whole; for each slide both copies have, its own settings (layout,
   background, transition, animations, notes, comments…) one by one, and its top-level shapes by
   `ShapeId`, added or replaced whole (a group with everything in it). A changed master (theme,
   layouts and all) is replaced whole.
   - **Text** travels as edits of its own wherever it is: a shape's own text, each table cell,
     the same for every shape inside a group (`collab/parts.rs`: the top-level shape's id with
     the inner shape's id and the cell), and the speaker notes (`SlideChange::notes`; the rest of
     the notes' settings as the `notes` setting, with their paragraphs as they were). Each edit
     (`collab/text.rs`, ported
     from WordLogic's stories): the paragraphs read as a flat sequence of characters (each with
     its run's formatting), breaks, fields and equations as one unit each, and a mark ending
     every paragraph with its settings. The edit keeps, inserts, deletes, and patches run
     formatting or a paragraph's settings, formatting as JSON maps of only the keys that
     changed; concurrent edits are transformed against each other as WordLogic's are. A shape
     whose other settings changed too is sent with its text as it was before, beside the text's
     edit, so the two merge with someone else's typing in either order. Applying rebuilds only
     the paragraphs an edit reaches; where that wouldn't give back the author's paragraphs
     exactly (runs the model keeps apart though they look alike), the shape is sent whole as
     before (`ShapesPatch::whole`).
3. **Presentation settings** (slide size, notes and handout masters, sections, custom shows, show
   settings, header and footer, properties, embedded fonts, table styles…) each replaced whole.
4. **Media** by `MediaId`: new or changed items travel with the op, as base64; a removal applies
   only where nothing uses the item any more (someone may have just put the same picture in).

Where two people change the same thing at once (a setting, a shape, a master, a slide's place),
the later one in the log wins it whole. A change to something someone else removed goes with
it. After applying, `normalize` puts right what two edits can leave behind together: a slide
whose layout went with a removed master takes the first layout, custom shows and sections drop
slides that are gone, a slide in no section joins its neighbour's, and a shape two people put in
two groups stays in the first (an emptied group goes). Commands never leave anything for it to
do (the fuzz checks), so it changes nothing on one person's own edits.

This covers every command without per-command work: paste, arrange, format, animations,
transitions and design changes are all results.

## 3. Ordering, ids, undo, joining

- **Server-ordered, client-transformed, one op in flight per client** (WordLogic's replica). The
  server numbers ops; log op `n`, made against position `base`, is rebased over entries
  `base+1 .. n-1` by every copy alike, so convergence needs only a deterministic `rebase` and
  `apply`. An op that `apply` refuses is a no-op everywhere.
- **What's shown is the truth for its person.** The next op is always the difference between
  what the server will have once the op in flight is in (`assumed`) and what's shown, so an op that
  landed differently than its author assumed is put right by the next one. A difference that
  would change nothing where it lands, or that every copy would refuse, isn't sent: what's shown
  takes the shared version of it (`Replica::take_refusal` says why, for telling the person).
- **Ids:** client `c` takes new ids from `0x4000_0000 + (c mod 1024) × 2²⁰` onwards, a range of
  2²⁰ of its own, above what files and new presentations use; `claim_ids` points the copy's
  `next_id` there, past any id an earlier session's client with the same range left. `next_id`
  is each copy's own and never travels. Slide ids in .pptx are renumbered on save anyway.
- **Undo is each person's own:** `Replica::local` keeps each edit's inverse (the diff back); it is
  rebased over the others' ops as they arrive (theirs win where both changed the same thing),
  and undoing sends it like any edit.
- **Joining:** a joiner loads the file, joins at the last checkpoint's log position and replays
  the entries after it. A .pptx read back numbers slides afresh, so when the checkpoint kept a
  state snapshot (the shared presentation in SlideLogic's native format, base64) the joiner
  takes that instead of the file it just opened. The page always fetches the file past the
  browser's cache: Nextcloud sends no `Cache-Control`, and a stale copy's old ETag made a joiner
  retry forever.

Known limits, accepted for now:
- A shape whose text couldn't travel as an edit (above) is replaced whole by the later change,
  and so is one whose text moved between places (a table that gained or lost rows or columns,
  a group that gained or lost shapes): someone typing in it at the same moment loses that
  typing. Two people changing the same setting of one shape (both moving it) take the later
  change.
- Copies on different builds in one session would read edits differently (a build from before
  text merging ignores text edits, and could save its copy over everyone's work). So each copy
  joins with `collab::PROTOCOL` (the browser's from `deckcraft:ready`'s `collab`), and the
  Nextcloud app sends any member on an older one than the newest that joined back
  (deckcraft-nextcloud 0.4.0; the protocol, not the app version, since 0.4.1): a browser tab
  reloads, which brings the newer editor. A copy that can't get newer that way says so instead
  of trying again: a desktop app older than the session opens the file unshared, and a page
  older than a desktop app in the session stops after one reload. A save the old copy had
  already started can still land in that moment.
- Two people changing one master (or adding layouts to it) at once: the later master wins whole;
  a slide on a layout that went takes the first layout.
- The diff can't tell which of two swapped neighbours was the one dragged: moving slide 1 below
  slide 2 and moving slide 2 above slide 1 are the same op. Fine alone; against a concurrent move
  of the same slide the "wrong" one may win. The session hook could pass the selection as a hint.
- Ranges repeat 1,024 clients apart (client ids grow by one per join), and a client that makes
  more than 2²⁰ ids in a session is refused further growth with a message.
- Only Undo and Redo read the session's snapshot history, and in a shared presentation they go to
  the replica, so no command takes back others' edits by restoring a snapshot.
- A snapshot over 128 MB (a very picture-heavy deck) isn't kept by the server; people joining
  from that save start from the .pptx, whose slide ids don't match the log's, and their copy
  would drift. Keep such presentations to one editor at a time.
- When the file changes outside the session, the member writing checkpoints stops at once; the
  others stop when their turn to write comes (within about a minute), as in WordLogic.
- The presentation shows as unsaved in the title until the next checkpoint.

## 4. Where the code goes

- `crates/engine/src/collab/`: `mod.rs` (op, capture, apply, rebase, normalize, ids), `seq.rs`
  (order edits), `replica.rs` (one person's copy), `shared.rs` (the session hook), `tests.rs`
  (scenarios and the fuzz).
- `crates/ui-egui/src/presence.rs`: the others on the canvas and the thumbnails.
- `apps/deckcraft-web/src/host.rs`, `collab_msg.rs`: the `deckcraft:collab:*` messages.
- `apps/deckcraft/src/nextcloud.rs`, `nextcloud/coedit.rs`: the desktop app's Nextcloud and its
  side of a session; `crates/ui-egui/src/cloud.rs`, `cloud_files.rs`: the File view's Nextcloud
  places. Tested end to end against a fake Nextcloud (`nextcloud/fake_server.rs`).
- `../deckcraft-nextcloud`: the sequencer (`CollabController`, `CollabService`, `SnapshotStore`,
  tables, clean-up job) and the page (`src/collab.js`), ported from WordLogic.

## 5. Phases

1. **Done (2026-10-18).** Ops and replay in the engine, collision-free ids, the replica with
   own-edit undo, and the convergence fuzz (two or three people, random commands, same final
   presentation, .pptx round trip).
2. **Done (2026-10-18).** The session hook, the bridge and the Nextcloud sequencer; joining from
   a checkpoint snapshot; presence (who's on which slide, others' selections).
3. **Done (2026-10-18).** Own-edit undo through the session, view-only members, checkpoints
   written by one member, shared links (files and folders), and the desktop app joining the same
   session (signed in to Nextcloud, presentations opened from the File view, or from the browser's
   Open in Desktop App through a `slidelogic://open` link). The Share dialog, as WordLogic's:
   in the browser through the page, on the desktop through its Nextcloud sign-in.
4. **Started (2026-10-18):** text merging wherever text is (shapes, groups, table cells, notes).
   Still to do: table rows and columns as edits of their own (so adding a row doesn't send the
   table whole); per-shape merging inside masters and layouts; per-property merging of one
   shape; a WebSocket transport.

## Revision history

| Date | Change |
|---|---|
| 2026-10-18 | Share dialog (browser and desktop) and Open in Desktop App (`slidelogic://open` links, registered by the installers) |
| 2026-10-18 | Phase 3 done: the desktop app signs in to Nextcloud, opens and saves presentations there, and co-edits with the browser; sessions keep to one co-editing protocol (`collab::PROTOCOL`) |
| 2026-10-18 | Notes drafts take in others' typing and save to the slide they were written on; a session keeps to its newest build |
| 2026-10-18 | Text merging in table cells, shapes inside groups and speaker notes, carets there following others' edits |
| 2026-10-18 | Text merging inside one shape: typing together, formatting and typing, a move and typing; carets and the current slide follow others' edits |
| 2026-10-18 | Shared links (files, folders) and a read-only user share checked end to end; a background tab no longer crashes before its first frame; view-only refusals read plainly and fit the status bar |
| 2026-10-18 | Phases 2–3 built: session hook, presence, web bridge, Nextcloud sequencer; checked end to end; status, joining, limits and phases updated |
| 2026-10-18 | Phase 1 built: `collab` module, replica, ids, fuzz; status, decisions, limits and phases updated |
| 2026-10-17 | Design (not started) |
