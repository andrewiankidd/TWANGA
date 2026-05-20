# TWANGA — GUI

The GUI is a single HTML + WASM bundle at [`frontend/web/`](../frontend/web/),
served two ways:

- **Web** — published to GitHub Pages at
  [andrewiankidd.github.io/TWANGA/app](https://andrewiankidd.github.io/TWANGA/app/).
  Same bundle the desktop shell hosts; no install needed.
- **Desktop (Tauri)** — [`crates/twanga-app/`](../crates/twanga-app/)
  wraps the same bundle in a native window. Work-paused while web parity
  is being proven first; the architecture is set up so any feature that
  ships on web ships unchanged in the desktop shell.

Feature parity with the CLI ([docs/CLI.md](CLI.md)) is enforced — every
flag on `twanga` has an equivalent GUI control, and vice versa. The GUI
is the better surface when you want screen real estate (multi-string
tab views, the Highway renderer, the Editor) or you'd rather not touch
a terminal.

![TWANGA GUI — main menu](../assets/screencaps/gui-menu.png)

## Screens

| Screen | What it does | CLI counterpart |
|--------|--------------|-----------------|
| **Tuner** | Live pitch detection vs your chosen tuning. Per-string display with capo. Chromatic mode if you pick "no instrument". | `twanga tune` |
| **Recorder** | Mic capture → fret detection → editable column grid → `.alphatex` download. Title prompt, BPM, resolution, block width, metronome, pre-roll, pause/resume. Mic-level meter surfaces "no signal" diagnostics. | `twanga record` |
| **Playback** | Library list (bundled examples + your IndexedDB recordings + drop-zone import). Per-tab view with the renderer host, BPM override, transpose, capo, loop, metronome, pre-roll, wait mode (with the same mic meter as the recorder). | `twanga play` |
| **Editor** | Post-capture cell-level edits to any tab in your library. Left-click a cell to bump fret up, right-click to bump down, double-click for direct numeric entry. Click a column index to select for insert/delete/clear. Saves back in place; bundled examples route through Save-as-new. | (GUI-only for now — CLI counterpart is a backlog item) |
| **Tunings** | Built-in + user-defined tunings merged into one list. Inline form to define new custom tunings; per-row Delete on user entries. | `twanga tunings list / add / remove` |

## Running it

### In the browser

Visit [the deployed app](https://andrewiankidd.github.io/TWANGA/app/).
Most modern browsers will work — desktop Safari, Chrome / Edge, Firefox.
First load fetches the WASM bundle (~few hundred KB); subsequent loads
hit the browser cache.

### Locally from source

```bash
# Build the WASM artifacts the frontend imports from ./pkg/
cd crates/twanga-web
wasm-pack build --target web --out-dir ../../frontend/web/pkg

# Serve frontend/web/ with any static server
cd ../../frontend/web
python -m http.server 8000
# then visit http://localhost:8000/app.html
```

The same `frontend/web/pkg/` is what the deployed Pages site uses (CI
runs `wasm-pack` on push). It's gitignored — every clone regenerates it
locally.

### As the Tauri desktop shell

Documented in [`crates/twanga-app/README.md`](../crates/twanga-app/README.md).
Work paused — the moment-to-moment plan is to land features on the web
build (instant iteration, same bundle 1:1) and then wire up Tauri once
the surface area is stable.

## Storage notes

The browser-only build stores user data in two places:

- **`localStorage`** — user-defined tunings, last-used picker state per
  screen, renderer plugin choice. Schema mirrors the CLI's
  `$CONFIG/twanga/tunings.toml` file format so a future Tauri sync
  command can read either side without translation.
- **IndexedDB** (`twanga-tabs-v1` / `tabs`) — recorded `.alphatex` blobs,
  imported drops, and the Editor's edits. Bundled examples ship with the
  app and aren't stored.

A **storage warning** sits at the top of the Recorder + Playback +
Editor screens reminding the user that browser storage can be evicted
("Clear browsing data", quota pressure). The Library exposes a
**Download** button per user entry so you can keep a real file copy;
each entry shows a "Backed up <when> / Never backed up" tag so you can
tell at a glance. The desktop (Tauri) shell reads from / writes to the
filesystem directly and hides this warning.

## Cross-tab sync

Open two browser tabs on the GUI, record in one, and the Library list
in the other refreshes automatically — same mechanism the Editor uses
to pick up new entries the Recorder just saved. Implemented via
`BroadcastChannel('twanga-tabs-changed')`; older browsers without
`BroadcastChannel` get a silent single-tab fallback.

## Renderer plugins

The Recorder, Playback, and Editor screens all share the same renderer
host: a small registry + plugin shape lives in
[`frontend/web/render/`](../frontend/web/render/). Two built-ins ship:

- **Tab** — column-grid notation, the same view as the CLI's
  scrolling tab. The Editor uses an `interactive: true` variant of
  this same plugin for cell-by-cell editing.
- **Highway** — Rocksmith-style falling notes, full-width playhead.

Both register through the same path a third-party plugin would. The
renderer picker on each screen is populated from the registry; there's
no special-casing for built-ins.

## Beyond this page

- **CLI counterpart** — [docs/CLI.md](CLI.md)
- **What works today** — [docs/PROJECT_STATUS.md](PROJECT_STATUS.md)
- **What's next** — [docs/ROADMAP.md](ROADMAP.md)
- **Tauri shell** — [crates/twanga-app/README.md](../crates/twanga-app/README.md)
- **WASM bindings** — [crates/twanga-web/README.md](../crates/twanga-web/README.md)
