# Real-time co-editing: design

> **Status (2026-10-18):** phase 1 done: ops, replay and convergence in the engine
> (`crates/engine/src/collab/`). Nothing reaches the user yet: the bridge messages, the
> Nextcloud sequencer and the session hook are phase 2. Until then two people saving the same
> file are kept apart by the Nextcloud app's ETag check (the second save is refused, nothing is
> lost). Fork-specific (Aegean Office), not upstream behaviour. GridLogic
> (`../gridcraft/docs/co-editing.md`) and WordLogic (`../wordcraft/docs/co-editing.md`) have the
> whole thing; SlideLogic follows them.

## Where it stands

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
- **Joining** (phase 2): load the last checkpoint plus the ops after it. A .pptx renumbers slide
  ids, so a joiner needs the checkpoint as the replica's confirmed presentation (the native
  format, or a snapshot as WordLogic keeps), not the re-read .pptx.

Known limits, accepted for now:
- One person typing in a text box while another changes the same shape: the later change wins
  the whole shape. Text merging inside a shape is phase 4.
- Two people changing one master (or adding layouts to it) at once: the later master wins whole;
  a slide on a layout that went takes the first layout.
- The diff can't tell which of two swapped neighbours was the one dragged: moving slide 1 below
  slide 2 and moving slide 2 above slide 1 are the same op. Fine alone; against a concurrent move
  of the same slide the "wrong" one may win. The session hook could pass the selection as a hint.
- Ranges repeat 1,024 clients apart (client ids grow by one per join), and a client that makes
  more than 2²⁰ ids in a session is refused further growth with a message.
- Some commands restore a snapshot from the local undo history (live previews). In a shared
  session the hook (phase 2) has to route Undo and Redo to the replica and keep these from
  taking back others' edits.

## 4. Where the code goes

- `crates/engine/src/collab/`: `mod.rs` (op, capture, apply, rebase, normalize, ids), `seq.rs`
  (order edits), `replica.rs` (one person's copy), `tests.rs` (scenarios and the fuzz). Done.
- `Session::collab_*` (phase 2): run every command through the replica (`local` after the
  outermost command, Undo and Redo to `Replica::undo`, refusals as a status message), as
  WordLogic's `collab/shared.rs`.
- `apps/deckcraft-web/src/host.rs`: the `deckcraft:collab*` messages, as GridLogic's bridge.
- `../deckcraft-nextcloud`: the sequencer (`CollabController`, `CollabService`, tables) and
  presence, ported from WordLogic for Nextcloud.

## 5. Phases

1. **Done (2026-10-18).** Ops and replay in the engine, collision-free ids, the replica with
   own-edit undo, and the convergence fuzz (two or three people, random commands, same final
   presentation, .pptx round trip).
2. The session hook, the bridge and the Nextcloud sequencer; joining from a checkpoint;
   presence (who's on which slide, others' selections).
3. View-only members, checkpoints written by one member, the desktop app.
4. Text merging inside one shape (WordLogic's flat sequence); per-shape merging inside masters
   and layouts; per-property merging of one shape.

## Revision history

| Date | Change |
|---|---|
| 2026-10-18 | Phase 1 built: `collab` module, replica, ids, fuzz; status, decisions, limits and phases updated |
| 2026-10-17 | Design (not started) |
