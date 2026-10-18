# Hosting SlideLogic for the web

`deckcraft-web-<version>.zip` (from the GitHub release, or `packaging/web/package.sh`) holds a
static site in `deckcraft-web-<version>/`:

| File | What it is |
|---|---|
| `index.html` | The page. It loads everything through relative URLs. |
| `deckcraft-web-<hash>.js` | wasm-bindgen glue (generated, ES module) |
| `deckcraft-web-<hash>_bg.wasm` | The app, size is printed by `packaging/web/package.sh` |
| `_headers`, `.htaccess` | Sample header rules for Netlify/Cloudflare Pages and Apache |

There is no server-side code. Upload the folder's contents anywhere that serves static files.

## Any path works

All URLs in `index.html` are relative (`public_url = "./"` in `apps/deckcraft-web/Trunk.toml`),
so the site works at a domain root (`https://example.com/`), under a prefix
(`https://example.com/tools/deckcraft/`) and from a CDN bucket. The asset names carry a content
hash, so they can be cached forever. Only `index.html` needs revalidation.

## Required server settings

- **MIME type:** serve `.wasm` as `application/wasm`. Browsers refuse to stream-compile it under
  any other type, and the app then loads slowly or not at all. Serve `.js` as `text/javascript`.
  Most hosts already do both. For nginx, check that `mime.types` has `application/wasm wasm;`.
- **Compression:** turn on gzip or Brotli for `.wasm`, `.js` and `.html`. That shrinks the
  download by roughly 60%. You can also precompress (`brotli -k *.wasm`) and let
  the server send `Content-Encoding: br`.
- **Caching:** `Cache-Control: public, max-age=31536000, immutable` on the hashed `.wasm` and
  `.js` files, and `no-cache` on `index.html`.
- **HTTPS:** WebGPU (and the clipboard) only work in a secure context, which means `https://`
  or `http://localhost`. Over plain HTTP elsewhere, the app falls back to WebGL2.
- **No special isolation headers:** SlideLogic doesn't use `SharedArrayBuffer`, so it doesn't
  need `Cross-Origin-Opener-Policy` or `Cross-Origin-Embedder-Policy`. If your site already sends
  COEP `require-corp`, also send `Cross-Origin-Resource-Policy: same-origin` (or `cross-origin`
  when the files live on a CDN) on the app's files.

nginx example:

```nginx
location /deckcraft/ {
    types { application/wasm wasm; text/javascript js; text/html html; }
    gzip on;
    gzip_types application/wasm text/javascript text/html;
    location ~* \.(wasm|js)$ { add_header Cache-Control "public, max-age=31536000, immutable"; }
    location ~* index\.html$ { add_header Cache-Control "no-cache"; }
}
```

Local test: `python3 -m http.server 8765` inside the folder, then open http://localhost:8765/.

## Embedding in a page (iframe)

```html
<iframe
  src="https://example.com/deckcraft/"
  title="SlideLogic presentations"
  style="width: 100%; height: 720px; border: 0;"
  allow="fullscreen; clipboard-read; clipboard-write"
  allowfullscreen>
</iframe>
```

- The app fills the iframe and follows its size, so size the iframe and not the app.
- Keyboard shortcuts go to the iframe after the user clicks into it, as with any embedded app.
- **Cross-origin embeds** work. The web build keeps no preferences between visits; it starts
  with the sample deck (or `?blank`) every time.
- **Sandboxed iframes** need at least
  `sandbox="allow-scripts allow-same-origin allow-downloads allow-popups"`. Without `allow-downloads`, Save and Export (browser
  downloads) are blocked.
- Don't send `X-Frame-Options: DENY` or a `frame-ancestors` CSP that excludes the embedding page.

## Opening and saving through the page (`?host=parent`)

With `?host=parent` in the iframe `src`, SlideLogic opens presentations the embedding page sends
it and hands the page every file it saves instead of downloading it. This suits a page with its
own storage, such as the SlideLogic app for Nextcloud. The page must be on the same origin as
SlideLogic: messages from anywhere else are ignored, and SlideLogic only answers its parent
window. In this mode SlideLogic starts with no presentation open (not the sample deck) until the
page sends one, and the page guards leaving it, from `deckcraft:dirty`.

Messages are plain objects passed with `postMessage`, each with a `type`:

| Message | Direction | Fields |
|---|---|---|
| `deckcraft:ready` | to the page | `version`; `ack` (true with `&ack=1`). Send the file to open after this. |
| `deckcraft:open` | from the page | `name`, `bytes` (`ArrayBuffer` or `Uint8Array`, up to 500 MB) |
| `deckcraft:save` | to the page | `name`, `bytes` (a transferred `ArrayBuffer`), `copy`; `id` with `&ack=1` when `copy` is false |
| `deckcraft:saved` | from the page (`&ack=1`) | `id`; optional `name` when the page stored the file under another name |
| `deckcraft:saveFailed` | from the page (`&ack=1`) | `id`, `error` (shown to the user) |
| `deckcraft:dirty` | to the page | `dirty`: whether any open presentation has unsaved changes (with `&ack=1`, including a save the page hasn't confirmed) |
| `deckcraft:failed` | to the page | `error`: the app couldn't start |

Other query flags: `&author=Name` signs new comments with the user's name.

**Opening.** `name` is the file's name; only its last path component is used. It must end in an
extension SlideLogic opens as a presentation (`.pptx`, `.potx`, `.ppsx`, `.deckcraft`); anything
else is ignored. Without a `name` the file opens as `Presentation.pptx`. Empty `bytes` open a new
blank presentation that saves under `name`, so a page can create a file and open it straight away.

**Saving.** Save sends the presentation (`copy: false`) under the name it was opened as, in its
own format (`.pptx`, `.potx`, `.ppsx`, `.deckcraft`); a presentation from anywhere else saves as
`.pptx` under the same name, and one that was never stored as `<title>.pptx`. Save As, Export (PDF,
pictures, GIF, outline) and Print send `copy: true`: files to store next to it, which don't change
what counts as saved.

**Save confirmation (`&ack=1`).** Without it, a presentation counts as saved as soon as SlideLogic
hands it to the page, even if the page then fails to store it. With it, every save with
`copy: false` carries an `id` and the page answers `deckcraft:saved` or `deckcraft:saveFailed`
with that `id`. Until it answers, `deckcraft:dirty` stays true. `deckcraft:saveFailed` leaves the
presentation unsaved and shows the user the page's `error`. Edits made after the handover stay
unsaved either way. When the page confirms a newer save of the same presentation, older saves
still waiting count as stored too, and a late answer to one of them changes nothing; a `name` in
`deckcraft:saved` is taken (so the next Save goes there) only when it has the same extension. A
page that leaves more than 64 saves unanswered has the oldest counted as failed. A SlideLogic that
doesn't know the flag answers `deckcraft:ready` without `ack: true`.

## Co-editing

Several people can edit one presentation at once (`docs/co-editing.md`). The page runs the session
with a server that numbers everyone's edits (for Nextcloud, `../deckcraft-nextcloud`); SlideLogic
does all the merging. `deckcraft:ready` says `collab: n` when the build can co-edit, `n` being
its co-editing protocol (`collab::PROTOCOL`: what its edits mean; the page joins with it, so a
session keeps to one). The messages follow WordLogic's, with the `deckcraft:` prefix:

| Message | Direction | Fields |
|---|---|---|
| `deckcraft:collab:start` | from the page | `client` (or `clientId`, nonzero), `seq`; optional `readOnly`, `user`, `color`, `state`. Sent after `deckcraft:open`: the presentation just opened is the shared one as of log position `seq`. With `state` (a checkpoint's snapshot) the snapshot is, and replaces the file just opened, since a `.pptx` read back numbers its slides afresh. `readOnly`: this person follows but can't edit |
| `deckcraft:collab:entries` | from the page | `json`: the server's log entries in order, `[{seq, client, client_seq, base, op}]`, `op` passed through untouched |
| `deckcraft:collab:op` | to the page | `json`: this person's next op, `{client, client_seq, base, op}`. The next one comes once this one is back in the log |
| `deckcraft:collab:checkpoint` | from the page | `id`: hand over the shared presentation to save |
| `deckcraft:collab:checkpointData` | to the page | `id`, `seq`, `name`, `bytes` (a transferred `ArrayBuffer`, the presentation as everyone has it, in the opened file's format), `state` (text: the snapshot for people who join from this save) |
| `deckcraft:collab:selection` | to the page | `json`: where this person is, `{slide, shapes}` (slide id, selected shape ids; `slide` is null in the master views), whenever it changes |
| `deckcraft:collab:presence` | from the page | `json`: the others, `[{client, name, color, slide, shapes}]`; their selected shapes are outlined on the slide in `color` (`#rrggbb`) with their `name`, and the slide's thumbnail shows their initials |
| `deckcraft:collab:stop` | from the page | the session ended; the presentation stays open, unshared |
| `deckcraft:collab:error` | to the page | `error`: this copy can't follow the log (an entry missing, a message it couldn't read) and must rejoin |

While shared, Undo and Redo take back only this person's own edits, and a view-only person's
edits are refused with a message. Co-editing messages are handled as they arrive, not at the next
frame, so a tab in the background keeps up and answers checkpoint requests. Log batches, snapshots
and presence lists are limited to 512 MB of text each.

## Sharing

With `?host=parent`, the title bar's Share button opens a Share dialog like PowerPoint's with
OneDrive (people with Can edit or Can view and a message, Copy link with the link's permission
in plain sight, link settings, who has access), and the page carries out its requests with the
user's own session (for Nextcloud, `../deckcraft-nextcloud/src/share.js`). Right-clicking Share
exports a copy instead; without a page to ask, Share opens File › Export.

| Message | Direction | Fields |
|---|---|---|
| `deckcraft:cloud` | to the page | `id`, `op`, `params` (JSON text): one of the dialog's operations (`crates/ui-egui/src/share.rs` lists them) |
| `deckcraft:cloud:reply` | from the page | `id`, `ok`; `json` (the answer, JSON text, at most 4 MB) when `ok`, else `error` |

A guest who opened a shared link gets Copy link for that link (`share.info` answers `link` and
`canShare: false`).

## Open in Desktop App

With `&desktop=1`, the title bar offers Open in Desktop App; choosing it sends
`deckcraft:openDesktop` (no fields) to the page, which opens the presentation in the SlideLogic
desktop app. For Nextcloud that's a `slidelogic://open?server=…&path=…` link: the desktop app
signs in to that server first if it needs to (Login Flow), then joins the same co-editing session.
The Windows installer and the Linux packages register the scheme
(`packaging/windows/deckcraft.wxs`, `packaging/linux/uk.co.aegeanlogic.slidelogic.links.desktop`);
macOS doesn't handle it yet. Each link opens its own window.

## Renderer selection and fallback flags

SlideLogic renders with wgpu. It uses **WebGPU** when the browser has it and falls back to
**WebGL2** on its own. URL query flags override this, and they work on the iframe `src` too:

| Flag | Effect |
|---|---|
| *(none)* | WebGPU if available, otherwise WebGL2 |
| `?webgl` | Force the WebGL2 backend (useful when a WebGPU driver misbehaves) |
| `?blank` | Start with an empty presentation instead of the sample deck |
| `?show` | Start the slide show immediately (kiosk/embedded decks) |

For example: `<iframe src="https://example.com/deckcraft/?webgl" ...>`.

A browser with neither WebGPU nor WebGL2 gets a message in place of the app.
