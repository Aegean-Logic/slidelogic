# SlideLogic app icon

**Banded slide**: a wide slide with an ink header band and a sea trace that forms an S and ends in
the aqua node, with three slide dots below (the current one in ink), on a marble tile. It is drawn
from the Aegean Logic palette so SlideLogic sits with GridLogic ("Sheet trace") and WordLogic ("W
trace") in the Aegean Office suite. It is original geometry written by hand for this repository,
deliberately not modelled on any vendor's presentation logo (no "P", no overlapping panels, no
projector screen).

Two masters:

- `deckcraft.svg`: the full artwork, used from 48 px up.
- `deckcraft-small.svg`: a simplified glyph for 32 px and below. It drops the slide dots, enlarges the
  slide, thickens the frame and trace and enlarges the node, so the icon still reads at 16 px.

## Palette

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#0A2A3B` | Slide frame, header band and the current slide's dot |
| Sea | `#12718C` | The trace |
| Aqua | `#2FC6C0` | The node at the end of the trace (one per icon) |
| Marble | `#F4F2EC` | The tile |
| White | `#FFFFFF` | The slide, and the ring around the node |
| Mist | `#CED4D8` | The other slide dots (full artwork only) |

## Geometry

A 512 × 512 viewBox; the tile is a rounded square with `rx=112`. Windows and Linux use the full
tile. The macOS icons (`.icns`, `deckcraft-macos-512.png`) sit on Apple's grid: an 824/1024
body with a transparent margin.

## Files

| File | Use |
|---|---|
| `deckcraft.svg` | Canonical artwork (48 px and up); also `hicolor/scalable` |
| `deckcraft-small.svg` | Small-size glyph (32 px and below) |
| `deckcraft-1024.png` | 1024 px render of the full tile |
| `deckcraft-macos-512.png` | Runtime Dock icon on macOS (for `apps/deckcraft` to embed) |
| `deckcraft.icns` | macOS bundle icon (`CFBundleIconFile`), 16–1024 px |
| `deckcraft.ico` | Windows exe icon and web favicon, 16–256 px (embedded by `apps/deckcraft/build.rs`) |
| `hicolor/<size>/apps/uk.co.aegeanlogic.slidelogic.png` | Linux icon theme, 16–512 px; the 256 px one is also the runtime window icon on Windows and Linux |
| `hicolor/scalable/apps/uk.co.aegeanlogic.slidelogic.svg` | Linux scalable icon |
| `LICENSE.txt` | MIT OR Apache-2.0 |

The app ID is `uk.co.aegeanlogic.slidelogic`. It's used for the hicolor icon names, the Wayland app
ID and `packaging/linux/uk.co.aegeanlogic.slidelogic.desktop`.

## Regenerate

Edit `deckcraft.svg` or `deckcraft-small.svg`, then run `packaging/icons.sh`. It needs `resvg`
(`cargo install resvg`) and runs on any platform: it packs the `.ico` with `cargo xtask ico` and
the `.icns` with `cargo xtask icns`.
