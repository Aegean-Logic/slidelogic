# Real-time co-editing: design (not started)

> **Status (2026-10-17):** design only. SlideLogic opens and saves presentations through the
> Nextcloud app (`../deckcraft-nextcloud`), and two people saving the same file are kept apart by
> its ETag check (the second save is refused, nothing is lost). Nothing below is built yet.
> GridLogic (`../gridcraft/docs/co-editing.md`) and WordLogic (`../wordcraft/docs/co-editing.md`)
> have it; this note says how SlideLogic would follow them.

## 1. What the engine gives us

- Every command runs through `Session::execute` and leaves the presentation as a new
  `Arc<Presentation>` (undo keeps the previous ones). The model is shared structure:
  `Presentation::slides` is `Vec<Arc<Slide>>`, masters are `Vec<Arc<Master>>`, and an edit
  replaces only the `Arc`s it touched. Comparing pointers finds what a command changed without
  looking inside the rest.
- Slides, layouts, masters, shapes and media have stable ids (`SlideId`, `LayoutId`, `MasterId`,
  `ShapeId`, `MediaId`). Ids are allocated per presentation, so two people adding a slide at once
  can pick the same one: ops need ids that can't collide (WordLogic §3: a per-client prefix).
- Selection and view state live in `DocState::selection`, outside the presentation, so they never
  need to travel.

## 2. Approach: send results, as GridLogic does

An op is the **result** of a local command, not the command:

1. **Slide order and membership**: slides added, removed or moved, by `SlideId` (and sections).
   These are the only edits that need transformation against concurrent ones (a move against a
   delete; two inserts at one place), as GridLogic's structural edits do.
2. **A content patch**: for each slide (and layout, master, notes page) whose `Arc` changed, the
   shapes that differ, keyed by `ShapeId`: added, removed, or replaced whole. Text inside a shape
   is replaced whole at first; per-paragraph merging (WordLogic's flat sequence) is a later step
   for two people typing in one text box.
3. **Presentation-level fields** (slide size, theme, header/footer, show settings, properties)
   replaced whole when they changed. Media bytes travel once, by `MediaId`, before the op that
   first uses them.

This covers every command without per-command work: paste, arrange, format, animations and
transitions are all patches.

## 3. Ordering, undo, joining

Reuse the GridLogic/WordLogic machinery rather than invent new: the Nextcloud app is the dumb
sequencer (sequence numbers, a log, presence), and each client rebases its unconfirmed ops on the
ops that arrive first. Undo takes back only the user's own ops (an inverse patch, skipped when
someone else has since changed the same shape). A late joiner loads the last checkpoint (the
file, saved every N ops or seconds by one member) plus the ops after it.

## 4. Where the code goes

- `crates/engine/src/collab/`: diff two presentations into an op, apply an op, transform the
  slide-order part, and the replica (local/unconfirmed/confirmed).
- `apps/deckcraft-web/src/host.rs`: the `deckcraft:collab*` messages, as GridLogic's bridge.
- `../deckcraft-nextcloud`: the sequencer (`CollabController`, `CollabService`, tables) and
  presence, ported from WordLogic for Nextcloud.

## 5. Phases

1. Ops and replay in the engine, with a convergence fuzz test (two sessions, random commands,
   same final presentation).
2. The bridge and the Nextcloud sequencer; presence (who's on which slide, others' selections).
3. Own-edit undo, view-only members, checkpoints.
4. Text merging inside one shape; the desktop app.
