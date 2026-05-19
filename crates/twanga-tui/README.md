# twanga-tui

Terminal UX primitives shared across TWANGA's CLIs — selection menus, refreshing displays, signal handling, ANSI colour helpers.

Cursor moves and line clears route through `crossterm` so Windows VT processing gets enabled the first time anything renders; after that, raw ANSI escape codes embedded in subsequent writes work too. Public surface:

- `select(prompt, options)` — 1-based numbered menu; reprompts on invalid input.
- `select_with_default(prompt, options, default_idx)` — same, but empty input picks the default (marked `*`).
- `prompt_parsed(prompt, default, parse_fn)` — free-form value with default shown in `[brackets]`. Accepts whatever the parser likes; reprompts on parse-failure (up to 3 attempts).
- `StatusLine` — single-line refreshing status display (used by the chromatic tuner).
- `MultiLineDisplay` — N-row refreshing block (used by the multi-string tuner, the tab recorder, and the playback cursor view).
- `spawn_line_reader()` — background thread that turns stdin lines into a `mpsc::Receiver<String>`. Lets long-running loops poll for `q + Enter`-to-quit input without enabling raw mode.
- `install_ctrl_c_handler()` / `is_shutdown_requested()` — graceful Ctrl-C handling. Install once near the start of `main()`, then poll in loops. Turns SIGINT into a clean process exit with status 0 (avoids `cargo run` reporting `STATUS_CONTROL_C_EXIT` / `0xc000013a` after every user-initiated stop).
- `color::{green, yellow, red, dim, reset}` — ANSI escape strings gated on a `bool` so callers can interpolate them safely in non-TTY output.
- `motd::print_banner()` — prints a fixed ASCII-art TWANGA logo plus a random splash from `twanga_core::SPLASHES` to stderr. Called once at the top of `main` so every subcommand gets the banner; writing to stderr keeps piped stdout (`twanga devices | grep USB`, `$(twanga tunings path)`) clean for scripted callers.

- **Check**: `cargo check -p twanga-tui`
- **Test**: `cargo test -p twanga-tui`
- **Depends on**: `twanga-core`, `crossterm`, `ctrlc`, `anyhow`
- **Used by**: `twanga-cli`

See [the workspace README](../../README.md) for project context.
