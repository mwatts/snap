# Snap -- Project Context for Claude Code

Always load the rust skill at the start of each session when working in this project.

## What this is

Snap is one Rust binary. It captures the screen, lets a human annotate it, and writes a PNG + JSON pair for coding agents.

There is no MCP server. There is no Tauri app. There is no Node. There is no Python.

## Four roles

The same `snap` binary is four roles. Role is a subcommand, not a second install.

| Role | How it starts | Needs daemon? |
|------|---------------|---------------|
| Daemon | `snap daemon` (launchd/systemd) or `snap start` | it is the daemon |
| Agent CLI | `snap check`, `latest`, `list`, `get`, `clear` | no |
| Project init | `snap init`, `snap use`, `snap projects` | no |
| Overlay | hotkey or `snap capture` | yes |

## Architecture

- One crate at the repo root. Edition 2024. `rust-version = "1.85"`.
- `src/main.rs` only parses CLI and calls `snap::cli::run()`.
- Agent commands read `<repo>/.snap/inbox/` on disk.
- The daemon is one process: a Unix socket thread plus a GPUI event loop.
- Last-active project in `~/.snap/state.toml` selects the inbox for a hotkey save.
- Overlay is GPUI. Composite uses `tiny-skia`.

## Key decisions

- Per-repo inbox at `<repo>/.snap/inbox/`. Never write `~/.snap/inbox/`.
- Agent commands work when the daemon is down.
- `snap latest` and `snap list` write `last_read`. `snap get` and `snap check` do not.
- Sidecar JSON matches the fork field names.
- Log to `~/.snap/snap.log`. Rotate at 1 MB. Do not log image bytes.
- macOS first. Linux capture adapters exist. Windows overlay is out of v1.

## Conventions

- CLI subcommands match the spec command surface.
- Domain errors that callers match use `thiserror`. The binary edge uses `anyhow`.
- Tests use a temp `HOME`. Do not touch the developer's real `~/.snap`.
- Git commits: short, direct messages.

## Build

```bash
cargo test
cargo build --release
make build          # cargo build --release
make install        # copy ~/.local/bin/snap and start snap daemon
```

Never `npx tauri build`. Never `cargo build` alone when you mean a release install. Use `cargo build --release` or `make build`.
