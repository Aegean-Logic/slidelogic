# Attribution

Every asset SlideLogic includes, bundles, embeds at build time or shows in its documentation, with
its author, source and licence. The policy is in [AGENTS.md](AGENTS.md) §1. `cargo xtask assets`
fails if a file under `assets/`, `docs/images/` or `docs/brand/` is missing from this table.

SlideLogic contains **no Microsoft, Adobe, Avid or Autodesk iconography, images, fonts, themes,
templates or other assets**. Its themes, colour schemes, layouts, preset-shape geometry, sample
deck and UI icons are original work of the SlideLogic and DeckCraft contributors (drawn or
generated in code). The one exception is the Aegean Logic brand files in `docs/brand/`: Aegean
Logic trademarks, not open source, used under `docs/brand/LICENSE-brand.txt`.
Icons placed from Insert › Icons are Lucide paths (ISC; Feather-derived icons MIT), not Microsoft
artwork. The ribbon and menu icons stay original drawings.

## In this repository

| Asset | Author | Source | Licence | Used for |
|---|---|---|---|---|
| `docs/brand/aegean-logic-lockup.svg` | Aegean Logic | Aegean Logic brand kit (primary lockup) | Aegean Logic brand terms (`docs/brand/LICENSE-brand.txt`) | Trademark, not open source; README header on light backgrounds |
| `docs/brand/aegean-logic-lockup-dark.svg` | Aegean Logic | Aegean Logic brand kit (reversed lockup) | Aegean Logic brand terms | Trademark; README header on dark backgrounds |
| `docs/brand/aegean-logic-mark.svg` | Aegean Logic | Aegean Logic brand kit (favicon mark) | Aegean Logic brand terms | Trademark; README footer |
| `docs/brand/aegean-logic-appicon.svg` | Aegean Logic | Aegean Logic brand kit (app icon, light) | Aegean Logic brand terms | Trademark |
| `docs/brand/LICENSE-brand.txt` | (licence text) | Aegean Logic | — | Terms for the Aegean Logic marks |
| `assets/app-icon/deckcraft.svg` | SlideLogic contributors | original (hand-written SVG in the Aegean Logic palette; `packaging/icons.sh`) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | SlideLogic app icon, "Banded slide" (48 px and up); master copied from the Aegean Office brand folder (`brand/slidelogic/slidelogic.svg`) |
| `assets/app-icon/deckcraft-small.svg` | SlideLogic contributors | original (hand-written SVG in the Aegean Logic palette; `packaging/icons.sh`) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | SlideLogic app icon, small-size glyph (32 px and below); from `brand/slidelogic/slidelogic-small.svg` |
| `assets/app-icon/deckcraft-1024.png` | SlideLogic contributors | rendered from deckcraft.svg by `packaging/icons.sh` (resvg) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, 1024 px render |
| `assets/app-icon/deckcraft-macos-512.png` | SlideLogic contributors | rendered from deckcraft.svg on Apple's icon grid by `packaging/icons.sh` (resvg) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, macOS Dock icon (embedded by `apps/deckcraft`) |
| `assets/app-icon/deckcraft.icns` | SlideLogic contributors | packed by `packaging/icons.sh` (resvg, `cargo xtask icns`) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, macOS bundle icon |
| `assets/app-icon/deckcraft.ico` | SlideLogic contributors | packed by `packaging/icons.sh` (resvg, `cargo xtask ico`; the small glyph at 32 px and below) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, Windows exe icon and installer |
| `assets/app-icon/hicolor/16x16/apps/uk.co.aegeanlogic.slidelogic.png` | SlideLogic contributors | rendered from deckcraft-small.svg by `packaging/icons.sh` (resvg) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, Linux hicolor render |
| `assets/app-icon/hicolor/24x24/apps/uk.co.aegeanlogic.slidelogic.png` | SlideLogic contributors | rendered from deckcraft-small.svg by `packaging/icons.sh` (resvg) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, Linux hicolor render |
| `assets/app-icon/hicolor/32x32/apps/uk.co.aegeanlogic.slidelogic.png` | SlideLogic contributors | rendered from deckcraft-small.svg by `packaging/icons.sh` (resvg) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, Linux hicolor render; web favicon |
| `assets/app-icon/hicolor/48x48/apps/uk.co.aegeanlogic.slidelogic.png` | SlideLogic contributors | rendered from deckcraft.svg by `packaging/icons.sh` (resvg) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, Linux hicolor render |
| `assets/app-icon/hicolor/64x64/apps/uk.co.aegeanlogic.slidelogic.png` | SlideLogic contributors | rendered from deckcraft.svg by `packaging/icons.sh` (resvg) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, Linux hicolor render |
| `assets/app-icon/hicolor/128x128/apps/uk.co.aegeanlogic.slidelogic.png` | SlideLogic contributors | rendered from deckcraft.svg by `packaging/icons.sh` (resvg) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, Linux hicolor render |
| `assets/app-icon/hicolor/256x256/apps/uk.co.aegeanlogic.slidelogic.png` | SlideLogic contributors | rendered from deckcraft.svg by `packaging/icons.sh` (resvg) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, Linux hicolor render; runtime window icon on Windows and Linux |
| `assets/app-icon/hicolor/512x512/apps/uk.co.aegeanlogic.slidelogic.png` | SlideLogic contributors | rendered from deckcraft.svg by `packaging/icons.sh` (resvg) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, Linux hicolor render |
| `assets/app-icon/hicolor/scalable/apps/uk.co.aegeanlogic.slidelogic.svg` | SlideLogic contributors | original (hand-written SVG in the Aegean Logic palette; `packaging/icons.sh`) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon, copy of deckcraft.svg |
| `assets/app-icon/README.md` | SlideLogic contributors | original | MIT OR Apache-2.0 | Icon documentation |
| `assets/app-icon/LICENSE.txt` | (licence text) | original | — | Licence of the app icon files |
| `packaging/macos/dmg/background.svg` | @XusBadia | original (hand-written SVG, derived from the SlideLogic app icon `assets/app-icon/deckcraft-1024.png` ("Banded slide"), referenced via `<image>`, not copied; Aegean Logic palette: ink #0A2A3B panel, marble #F4F2EC field, sea #12718C rules, one small aqua #2FC6C0 accent; text is outlined Inter / JetBrains Mono glyph paths) | MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`) | SlideLogic macOS DMG window background for the Finder window; `packaging/macos/dmg/generate.py` renders it to `background.tiff`; added 2026-10-08 (DeckCraft), restyled to the Aegean Logic palette with the SlideLogic name 2026-10-18 |
| `packaging/macos/dmg/background.tiff` | @XusBadia | Rendered from `packaging/macos/dmg/background.svg` by `packaging/macos/dmg/generate.py` | MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`) | 1x + 2x HiDPI TIFF for the macOS DMG window; includes the app icon (see its row) and LittleCMS's built-in sRGB profile ("No copyright, use freely") |
| `packaging/windows/branding/banner.svg` | SlideLogic contributors | original (hand-written SVG; references the Aegean Logic reversed lockup `docs/brand/aegean-logic-lockup-dark.svg` via `<image>`, not copied; Aegean Logic palette: ink #0A2A3B into sea #12718C, marble #F4F2EC text set in the system's Segoe UI when rendered, one aqua #2FC6C0 accent; as GridLogic's and WordLogic's) | MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`); the lockup it shows is under the Aegean Logic brand terms (`docs/brand/LICENSE-brand.txt`) | Windows installer banner, top of every SlideLogic Setup dialog; added 2026-10-18 |
| `packaging/windows/branding/banner.bmp` | SlideLogic contributors | Rendered from `packaging/windows/branding/banner.svg` by `packaging/windows/branding/generate.ps1` (resvg, then 24-bit BMP) | MIT OR Apache-2.0; includes the Aegean Logic lockup (Aegean Logic brand terms) | Windows installer banner, 833 x 137 px (370 x 56 dialog units) |
| `docs/images/editor.png`, `docs/images/shape-format.png`, `docs/images/format-pane.png`, `docs/images/slide-sorter.png`, `docs/images/dark-mode.png`, `docs/images/sample-deck.png` | DeckCraft contributors | Screenshots of DeckCraft and slides rendered by `deckcraft-cli` from the built-in sample deck (original content; Inter font, OFL-1.1) | MIT OR Apache-2.0 | README screenshots |
| `crates/media/tests/fixtures/tone*` (`tone.mp3`, `.m4a`, `.aac`, `.ogg`, `.opus`, `.flac`, `.wma`, `tone-alac.m4a`) | DeckCraft contributors | synthesised 0.5 s 440 Hz sine, encoded by `crates/media/tests/fixtures/make.sh` (ffmpeg as the encoder only) | CC0-1.0 | Audio decoding tests |
| `crates/media/tests/fixtures/clip-*` (`clip-h264.mp4`, `clip-h264.mov`, `clip-hevc.mp4`, `clip-vp9.webm`, `clip-av1.mkv`) | DeckCraft contributors | 6 frames of a colour gradient generated by `make.sh` (python3), encoded by ffmpeg (x264 / x265 / libvpx / SVT-AV1 as encoders only) | CC0-1.0 | Video decoding tests |
| `crates/media/tests/fixtures/damaged/*` (`ebml-header.mkv`, `ebml-position.webm`, `demux-loop.mp4`, `demux-loop.m4a`) | DeckCraft contributors | the `clip-*` and `tone*` fixtures above with bytes changed by a mutation fuzzer | CC0-1.0 | Damaged media must not panic or hang |
| `crates/media/tests/fixtures/damaged/*` (`stsc-count.m4a`, `stsz-size.m4a`) | DeckCraft contributors | `tone.m4a` above with bytes changed by a mutation fuzzer | CC0-1.0 | Damaged MP4 sample tables must not abort |
| `crates/media/tests/fixtures/make.sh` | DeckCraft contributors | this repository | MIT OR Apache-2.0 | Regenerates the media fixtures |
| `crates/render/tests/fixtures/colr-squares.ttf` | DeckCraft contributors | two squares as a COLRv0/CPAL test font, built by `make_colr_font.py` (fontTools as the builder only) | CC0-1.0 | Colour glyph rendering tests |
| `crates/fonts/tests/fixtures/legacy-semibold.ttf` | DeckCraft contributors | a one-glyph Semibold face named like Segoe UI Semibold (its Windows family differs from the typographic one), built by `make_legacy_font.py` (fontTools as the builder only) | CC0-1.0 | Font lookup by Windows family name tests |
| `crates/fonts/tests/fixtures/legacy-regular.ttf` | DeckCraft contributors | the same family's one-glyph Regular face, built by `make_legacy_font.py` (fontTools as the builder only) | CC0-1.0 | Font lookup by Windows family name tests |
| `crates/render/tests/fixtures/make_colr_font.py` | DeckCraft contributors | this repository | MIT OR Apache-2.0 | Regenerates the COLR test font |
| Lucide icon paths in `crates/engine/src/icon_data.rs` (Insert › Icons) | Lucide Contributors; Feather icons by Cole Bemis | https://github.com/lucide-icons/lucide/tree/440a1e1dfb1e712505308da46bead3c28b1f1398/icons (release 1.56.0, 2026-10-13) | ISC; MIT for the Feather-derived icons named in that commit's LICENSE | Stroke icons inserted as shapes |

## Bundled through dependencies

| Asset | Author | Source | Licence | Used for |
|---|---|---|---|---|
| Ubuntu Light (`Ubuntu-Light.ttf`, via the `epaint_default_fonts` crate 0.36) | Dalton Maag Ltd for Canonical | https://crates.io/crates/epaint_default_fonts | Ubuntu Font Licence 1.0 | Last-resort text face when no other font is available; UI fallback |
| Hack Regular, Noto Emoji, emoji-icon-font (via `epaint_default_fonts` / egui) | Source Foundry; Google; Jakub Steiner et al. | https://crates.io/crates/epaint_default_fonts | MIT / Bitstream-Vera, OFL-1.1, MIT | egui UI fallback glyphs |

## Embedded at build time from craft-fonts (`CRAFT_FONTS_DIR`)

Release builds (and local builds with a craft-fonts checkout) embed the fonts listed in
[craft-fonts](https://github.com/storytold/craft-fonts) `fonts/latin-manifest.txt` and
`fonts/manifest.txt` (per-file authors, sources, SHA-256 and licences in its
[ATTRIBUTION.md](https://github.com/storytold/craft-fonts/blob/main/ATTRIBUTION.md)), except the
families excluded by `crates/fonts/build.rs`:

| Families | Licence | Notes |
|---|---|---|
| Inter | OFL-1.1 | Default theme font |
| Carlito, Caladea | OFL-1.1 | Metric-compatible stand-ins for documents that ask for Calibri / Cambria |
| Liberation Sans, Serif, Mono | OFL-1.1 | Metric-compatible stand-ins for Arial, Times New Roman, Courier New |
| Montserrat, Lato, Open Sans, Roboto, Merriweather, Playfair Display, Poppins, Nunito Sans | OFL-1.1 | Theme fonts |
| BIZ UDPGothic, BIZ UDMincho, Shippori Mincho, Noto Sans Arabic | OFL-1.1 | Japanese and Arabic text |

Web (wasm32) builds embed only Inter Regular/Bold and BIZ UDPGothic Regular.

## PDF writer colour profiles

`craft-pdf` is used with only its `write` feature and default features disabled. Krilla
0.8.2 embeds the following profiles; these were already reachable through the direct Krilla
exporter. No interpreter fallback fonts or CMaps are enabled, and no profile or font binary
is copied into this repository. The licence text is in `licenses/pdf-writer/LICENSE-ICC.txt`.

| Asset | Author | Source | Licence | SHA-256 | Used for |
|---|---|---|---|---|---|
| `krilla 0.8.2 / icc/sGrey-v2-magic.icc` | Clinton Ingram (Compact ICC Profiles) | https://github.com/LaurenzV/krilla/blob/3ffdf0588cf98050aad6edba51ca70162e1fb5b5/crates/krilla/icc/sGrey-v2-magic.icc | CC0-1.0 | `4110169287e5f1b5149ca9f1da1814d32943905bba8030e849d1a3b729a19481` | PDF export colour profiles |
| `krilla 0.8.2 / icc/sGrey-v4.icc` | Clinton Ingram (Compact ICC Profiles) | https://github.com/LaurenzV/krilla/blob/3ffdf0588cf98050aad6edba51ca70162e1fb5b5/crates/krilla/icc/sGrey-v4.icc | CC0-1.0 | `00c0f94e09127520a17dc0e1d9264b5702081d96dbdb1549ee88e3631ce42a9d` | PDF export colour profiles |
| `krilla 0.8.2 / icc/sRGB-v2-magic.icc` | Clinton Ingram (Compact ICC Profiles) | https://github.com/LaurenzV/krilla/blob/3ffdf0588cf98050aad6edba51ca70162e1fb5b5/crates/krilla/icc/sRGB-v2-magic.icc | CC0-1.0 | `af4efe28f6d311799865f325ba39184a2e978b113ca74124d60af7de22b105f4` | PDF export colour profiles |
| `krilla 0.8.2 / icc/sRGB-v4.icc` | Clinton Ingram (Compact ICC Profiles) | https://github.com/LaurenzV/krilla/blob/3ffdf0588cf98050aad6edba51ca70162e1fb5b5/crates/krilla/icc/sRGB-v4.icc | CC0-1.0 | `c56e1685d888f5edb92fe07f2750f387f8fe8e91b32ff8fb0b56bfbbb9458353` | PDF export colour profiles |
