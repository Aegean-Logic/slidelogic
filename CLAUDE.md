# SlideLogic (DeckCraft fork) — working notes for Claude

This is Aegean Logic's fork, shipped as SlideLogic (see the fork note at the top of AGENTS.md).
User-visible text says SlideLogic; internal identifiers stay `deckcraft`. Commit on the fork's
branches; push to `origin` only when the user asks, never to `upstream`, with the work email.

Read [AGENTS.md](AGENTS.md) first: its rules (asset policy, clean room, never crash) bind you and
win over anything here.

## Start every session
1. Read `plan/STATUS.md`, then the current milestone in `plan/execution-plan.md`.
2. Read `../../craftrules/AGENTS.md` and the standards it lists for your task.
3. Run `cargo xtask ci` before you change anything, so you know the tree is green.

## Habits
- Every user-visible action is a command with tests (`crates/engine/src/cmd/*`).
- Look at UI changes: `deckcraft --sample --control 7990` + `ui.screenshot`, or
  `deckcraft-cli render`.
- Commit after each landed arc (`M<n>.<k>: what`) and push to `origin main`.
- Keep `ROADMAP.md`, the progress docs in `docs/` (AGENTS.md §7) and `plan/STATUS.md` current.
- Don't ask the user questions; record genuinely-theirs decisions in `plan/STATUS.md`.
