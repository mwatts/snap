# Snap Rust rewrite

Date: 2026-08-18
Branch: `feat/rust`
Status: approved for spec; not implemented

## 1. Problem
Snap is a fork of Adjective LLC's annotation tool. The fork is a Tauri 2 app (Rust + vanilla JS) plus a Python MCP server. Agents read `~/.snap/inbox/` through MCP tools.
That stack is too large for the product. The install needs Node, a WebView, Python, and `fastmcp`. The agent interface is MCP, not a command.
This rewrite makes one Rust binary named `snap`. That binary is the daemon, the agent CLI, the project init tool, and the overlay.

## 2. Interface boundary
The product is the `snap` executable. There is no MCP server, no Node, no Tauri, and no Python.
Humans talk to the binary through a global hotkey and a GPUI overlay. Agents talk to the binary through subcommands. The daemon talks to a second `snap` process through a Unix socket.

## 3. Review of the fork
### What the fork does well
- The human loop is short: `Ctrl+Shift+S`, draw, Enter, inbox pair.
- The sidecar JSON is simple and useful to an agent.
- Capture and window context already branch by OS.
- The inbox is a queue, not an archive. `clear_inbox` is part of the loop.
- Logging goes to `~/.snap/snap.log` and rotates at 1 MB.

### What the fork costs
- `app/` is a Tauri 2 project. A release build embeds a WebView and a JS canvas (`main.js` is about 1,100 lines).
- `mcp-server/` is a second runtime (`fastmcp` on stdio). Agents must register that server in each editor.
- Tray mode and overlay mode are two lifecycles in `main.rs`. Wayland needs `snap-trigger.sh`. X11/macOS need a tray process.
- Save sends a base64 PNG through Tauri IPC. The JS canvas also has a tainted-canvas workaround (`read_capture_base64`).
- Hard-coded paths appear in `snap-trigger.sh` and `snap.service` (`$HOME/Desktop/snap/...`).
- Tests cover crop mapping and MCP image inlining. The drawing engine has no unit tests.

### What this rewrite keeps
- Annotation tools: circle, rectangle, arrow, freehand, text, numbered marker.
- Colors, stroke widths, dim, undo, clear, save, cancel.
- Keyboard map (see §9).
- Sidecar schema (see §8).
- Window context fields when the OS can give them.
- Log path and 1 MB rotation.
- macOS first. Linux capture adapters stay in the design. Windows overlay is out of v1.

### What this rewrite drops
- Tauri, WebView, Node, `app/`
- Python, `fastmcp`, `mcp-server/`, `setup-mcp.sh`
- MCP tool names as the agent API
- A global inbox at `~/.snap/inbox/`
- Tray icon in v1
- Rebindable hotkey UI in v1

## 4. Goals
- One crate. One binary. Four roles (see §6).
- Per-repo inbox at `<repo>/.snap/inbox/`.
- Last-active project selects the inbox for a hotkey capture.
- Agent commands read the inbox on disk. They do not need the daemon.
- Overlay is GPUI. Composite uses `tiny-skia`.
- Old `app/` and `mcp-server/` stay on the branch until one overlay save and `snap latest` work. Then they are deleted on the same branch.

## 5. Non-goals (v1)
- MCP, including a later `snap mcp` stdio wrapper
- Windows overlay polish
- Wayland trigger script parity
- Tray icon
- Hotkey settings UI
- Auto-migrate of `~/.snap/inbox/`
- Cloud, network, or telemetry
- Image edit beyond the six tools above

## 6. Binary roles and commands
The binary is `snap`. Role is a subcommand, not a second install.

| Role | How it starts | Needs daemon? |
|------|---------------|---------------|
| Daemon | `snap daemon` (launchd/systemd) or `snap start` | it is the daemon |
| Agent CLI | `snap check`, `latest`, `list`, `get`, `clear` | no |
| Project init | `snap init`, `snap use`, `snap projects` | no (init/use also update last-active; `use` tells the daemon if it is up) |
| Overlay | hotkey or `snap capture` | yes |

### Command surface

```
# lifecycle
snap daemon              # this process is the daemon (GPUI event loop)
snap start               # start daemon if a ping fails
snap stop                # ask the daemon to shut down
snap status              # daemon up, socket, last-active project
snap doctor              # permissions, capture tool, socket, last-active path

# project
snap init                # create .snap/, register repo, patch agent markdown
snap use [path]          # set last-active project (default: this repo)
snap projects            # list registered repos and mark the active one

# capture
snap capture             # ask the daemon to open the overlay

# agent (JSON on stdout unless noted)
snap check
snap latest
snap list [n]            # default n = 5
snap get <id>            # id is the filename stem, e.g. snap-20260818-142015-897
snap clear
```

`--json` is the default for agent commands. Human-readable text is allowed on `status`, `doctor`, `projects`, `init`, `start`, `stop`.
Exit code 0 means success. Exit code 1 means a user or environment error (no repo, empty inbox, daemon down). Exit code 2 means a usage error.

## 7. Process and IPC
### Process model
`snap daemon` is one GPUI process. It has no window until a capture. On macOS it uses the accessory activation policy (no Dock bounce).
A background thread owns the Unix socket. The GPUI thread owns the overlay and the global hotkey.
`snap start` pings the socket. If the ping works, it prints that the daemon is up and exits 0.
If the ping fails, it starts `snap daemon` as a detached child. It then waits for a ping, or 3 seconds.
`snap stop` sends `shutdown` on the socket. If the socket is dead, it removes a stale pid/socket pair.
Only one daemon is allowed. A second `snap daemon` pings first and exits if the peer answers.

### Paths (global)
```
~/.snap/snap.sock      # Unix socket
~/.snap/snap.pid       # daemon pid
~/.snap/snap.log       # event log
~/.snap/snap.log.old   # previous log after rotate
~/.snap/state.toml     # last-active project
~/.snap/projects.toml  # registry of inited repos
```
On Linux, if `XDG_RUNTIME_DIR` is set, the socket is `$XDG_RUNTIME_DIR/snap.sock`. The pid and log stay under `~/.snap/`.

### Socket protocol
Each client opens the socket, writes one JSON object, writes a newline, then reads one JSON object. Then it closes the connection.
Requests:

```json
{"op":"ping"}
{"op":"capture"}
{"op":"use","path":"/abs/path/to/repo"}
{"op":"status"}
{"op":"shutdown"}
```

Responses:

```json
{"ok":true,"data":{}}
{"ok":false,"error":"no active project"}
```

`status` data:

```json
{
  "pid": 12345,
  "active_project": "/abs/path/to/repo",
  "active_name": "snap",
  "socket": "/Users/you/.snap/snap.sock"
}
```

The daemon never reads or writes inbox PNG/JSON for agent commands. Agent commands use the filesystem.

### Last-active project
`~/.snap/state.toml`:

```toml
active_project = "/abs/path/to/repo"
```

`snap init` and `snap use` write this file. If the daemon is up, they also send `use` on the socket so the in-memory copy matches.
The overlay shows the project name as a chip. A click on the chip cycles registered projects that still exist on disk. The cycle updates `state.toml` and the in-memory copy before save.

### How a hotkey picks a repo
1. The daemon reads the in-memory last-active path.
2. If that path has `.snap/config.toml`, the overlay opens and save writes to that repo's inbox.
3. If the path is missing or has no `.snap/`, the overlay does not open. The daemon logs the error. `snap doctor` explains the fix: `snap init` or `snap use`.

The frontmost window is not used to pick the repo. A browser has no repo cwd. Window context still goes into the sidecar `source` object.

## 8. Inbox, schema, and `snap init`
### Per-repo layout
```
<repo>/
  .snap/
    config.toml      # committed
    .gitignore       # ignores inbox/ and last_read
    last_read        # local, unix seconds as text
    inbox/
      snap-<YYYYMMDD>-<HHMMSS>-<mmm>.png
      snap-<YYYYMMDD>-<HHMMSS>-<mmm>.json
  AGENTS.md          # created or patched
```

`.snap/config.toml`:

```toml
[project]
name = "snap"
```

The name defaults to the directory name. `inbox/` and `last_read` are never committed. `config.toml` is committed so a clone knows the repo uses Snap.

### Global registry
`~/.snap/projects.toml`:

```toml
[[projects]]
path = "/abs/path/to/repo"
name = "snap"
inited_at = "2026-08-18T14:20:15Z"
```

### Sidecar JSON
The write format matches the fork. Example:

```json
{
  "timestamp": "2026-08-18T14:20:15.897Z",
  "source": {
    "window_title": "localhost:3000/dashboard - Brave",
    "url": null,
    "window_class": "Brave-browser",
    "pid": 12345,
    "display": "primary",
    "resolution": [1920, 1080]
  },
  "annotations": [
    {
      "type": "circle",
      "center": [340, 220],
      "radius": [45, 45],
      "color": "#FF3B30",
      "label": null
    }
  ],
  "image_filename": "snap-20260818-142015-897.png"
}
```

Annotation variants (same as the fork):

| type | fields |
|------|--------|
| `circle` | `center`, `radius`, `color`, `label` |
| `rect` | `position`, `size`, `color`, `label` |
| `arrow` | `from`, `to`, `color`, `label` |
| `freehand` | `points`, `color`, `label` |
| `text` | `position`, `content`, `color` |
| `marker` | `position`, `number`, `color` |

Coordinates are overlay logical pixels. Export scales them to native capture pixels. The scale algorithm is the same as `mapCropToNative` in `app/src/export-scale.mjs`.

### Agent read shape
`snap latest`, `snap get`, and each item from `snap list` add `filename` and `image_path`. They do not inline base64 in v1. The agent reads the PNG from `image_path`.

```json
{
  "filename": "snap-20260818-142015-897",
  "image_path": "/abs/repo/.snap/inbox/snap-20260818-142015-897.png",
  "timestamp": "2026-08-18T14:20:15.897Z",
  "source": {},
  "annotations": [],
  "image_filename": "snap-20260818-142015-897.png"
}
```

`snap check` compares each pair's mtime to `.snap/last_read`. It does not write `last_read`.
`snap check` stdout:

```json
{
  "has_new": true,
  "new_count": 2,
  "new_annotations": ["snap-20260818-142015-897"],
  "total_in_inbox": 5
}
```

`snap list [n]` writes a JSON array of the read shape above, newest first. Default `n` is 5.
`snap latest` and `snap list` write `last_read` to now after a successful read.
`snap get` does not write `last_read` (same as the fork's `get_annotation`).
`snap clear` deletes all `snap-*.json` / `snap-*.png` pairs in that inbox and removes `last_read`.

### Repo resolve for agent commands
1. Walk from cwd toward parent dirs. Stop at the first `.snap/config.toml`.
2. If none, use `active_project` from `~/.snap/state.toml`.
3. If that path is gone or has no `.snap/`, exit 1 with a message that names `snap init` and `snap use`.

### `snap init` steps
1. Find the git root. If there is no git root, use cwd.
2. Create `.snap/config.toml`, `.snap/inbox/`, and `.snap/.gitignore`.
3. Append or update the project in `~/.snap/projects.toml`.
4. Set `active_project` in `~/.snap/state.toml`.
5. If the daemon is up, send `use` with the repo path.
6. Patch agent markdown. Prefer `AGENTS.md` if it exists. Else use `CLAUDE.md` if it exists. Else create `AGENTS.md`.
7. The markdown block tells the agent to call `snap check` and `snap latest`, read the PNG, and interpret the six tools.
8. The command is idempotent. A second run does not duplicate the markdown block.

The markdown block is wrapped in HTML comments:

```markdown
<!-- snap-agent-block:start -->
...
<!-- snap-agent-block:end -->
```

## 9. Overlay, tools, and capture
### Overlay
The overlay is a fullscreen, undecorated, always-on-top GPUI window. The capture is the background. The toolbar is a custom GPUI view (use `gpui-base` so the app owns the look). The project chip is in the toolbar.
GPUI channel: git `zed-industries/zed` plus `gpui_platform` (not crates.io `0.2.2`). Pin a commit. Bootstrap with `gpui_platform::application()`. Call `gpui_base::init` if the toolbar uses `gpui-base`. Do not also call `gpui_component::init`.

### Tools and keys (parity)

| Tool | Key | Gesture |
|------|-----|---------|
| Circle | `C` | drag; Shift makes a circle |
| Rectangle | `R` | drag; Shift makes a square |
| Arrow | `A` | drag; head at the end |
| Freehand | `F` | drag; simplify points with distance tolerance 2 |
| Text | `T` | click, type, Enter |
| Marker | `N` | click; numbers increment from 1 |

| Key | Action |
|-----|--------|
| `Ctrl+Shift+S` | global hotkey; open overlay |
| `D` | toggle dim (30% black) |
| `Ctrl+Z` / `Cmd+Z` | undo last annotation |
| `Enter` | save and close |
| `Escape` | cancel in-progress draw, else discard and close |

Toolbar also has six colors (`#FF3B30`, `#007AFF`, `#34C759`, `#FFCC00`, `#FFFFFF`, `#000000`), three widths (2, 4, 6), dim, undo, clear, save. The toolbar is draggable.
Default tool is circle. Default color is `#FF3B30`. Default width is 4.

### Platform capture
Get window context **before** the overlay is shown.

| OS | Capture | Region | Window context |
|----|---------|--------|----------------|
| macOS | `screencapture -i -s <tmp>` | interactive system picker; cancel means no overlay | AppleScript via `osascript` (title, app, URL, pid) |
| Linux | `gnome-screenshot`, then `grim`, then `scrot` | full capture, then in-overlay drag select (min 10×10) | `xdotool` on X11; empty fields on Wayland |
| Windows | out of v1 | — | — |

Temp file: `std::env::temp_dir().join("snap-capture.png")`.
macOS needs Screen Recording and Accessibility. The binary Info.plist (or equivalent entitlements) keeps the usage strings from `app/src-tauri/Info.plist`. `snap doctor` tells the user if a permission is missing.

### Composite
`tiny-skia` draws the capture (cropped if needed) and then the annotations at native capture resolution. Text uses a font file that ships in `assets/` so export does not depend on a system font. Output is PNG. Save writes the PNG and the sidecar in the last-active inbox. Then the overlay closes.

### Capture session (happy path)
1. Hotkey or `snap capture` fires.
2. Daemon checks last-active `.snap/config.toml`.
3. Daemon records window context.
4. Daemon captures the screen (macOS interactive, else full).
5. Daemon opens the overlay window.
6. User annotates (Linux may crop first).
7. Enter writes PNG + JSON to `<repo>/.snap/inbox/`.
8. Overlay closes. The daemon stays up.

## 10. Errors, logging, and limits
### Errors
- Recoverable work returns `Result`. The CLI prints one line to stderr.
- The daemon does not panic on a bad client, a failed capture, or a missing project. It logs and stays up.
- Capture failure: show a 3 second message on a small overlay, then close.
- Save failure: keep the overlay open and show a banner.
- Stale socket or pid: `snap start` deletes them and starts a new daemon.
- Empty inbox: `snap latest` / `get` exit 1 with `{"error":"No annotations in inbox"}`.
- Unknown id: `snap get` exits 1 with `{"error":"Annotation '<id>' not found"}`.

### Logging
Use `tracing` to `~/.snap/snap.log`. Rotate at 1_000_000 bytes to `snap.log.old`. Log event names and paths. Do not log image bytes.

### Numeric targets
- Agent commands (`check`, `latest`, `list` with n ≤ 20) finish in less than 100 ms for an inbox with fewer than 100 pairs.
- Socket ping round trip is less than 20 ms on the local host.
- `snap start` waits at most 3_000 ms for the first successful ping.
- Capture-failure overlay stays up for 3_000 ms.
- Log rotate threshold is 1_000_000 bytes.

## 11. Testing
v1 tests are `cargo test` with no GPU.

| Area | What to test |
|------|----------------|
| Inbox | write pair, list newest first, get by id, clear, missing png warning |
| Check | `last_read` cursor, new vs old mtime |
| Registry | init twice is idempotent, `use` changes last-active, missing path errors |
| Resolve | cwd walk finds `.snap/`, else last-active, else error |
| Schema | each annotation variant round-trips |
| Crop | `map_crop_to_native` matches the two cases in `export-scale.test.mjs` |
| Draw | undo pops, marker counter decrements, simplify_points keeps ends |
| IPC | fake daemon thread answers ping/status/use/shutdown |
| Init markdown | inserts block, second run does not duplicate |

Capture and window context are traits. Tests use in-memory fakes. GPUI tests (`#[gpui::test]`) are optional and are not a v1 gate.

## 12. Crate layout
One crate at the repo root. Edition 2024. Application errors use `anyhow` at the binary edge. Domain errors that callers match (`InboxError`, `IpcError`) use `thiserror`.

```
snap/
  Cargo.toml
  Cargo.lock
  src/
    main.rs          # clap dispatch only
    cli.rs
    daemon.rs        # socket thread + gpui::run
    ipc.rs
    state.rs         # state.toml + projects.toml
    inbox.rs
    project.rs       # init, use, resolve
    capture.rs       # ScreenCapture trait + OS adapters
    window.rs        # WindowContext trait + OS adapters
    overlay.rs       # GPUI window and toolbar
    canvas.rs        # tools, gestures, live preview
    draw.rs          # annotation types + tiny-skia composite
    logutil.rs
  assets/
    font.ttf         # bundled for export
  docs/superpowers/specs/2026-08-18-rust-rewrite-design.md
```

`main` only loads config, starts logging, and calls `cli::run`. It does not import capture OS crates or GPUI types beyond the daemon path.

Suggested crates: `clap`, `serde`, `serde_json`, `toml`, `anyhow`, `thiserror`, `tracing`, `tracing-subscriber`, `dirs`, `chrono`, `png` / `image`, `tiny-skia`, `global-hotkey`, `gpui` + `gpui_platform` (git, pinned), `gpui-base` (git, optional).

## 13. Migration
1. Add the new crate at the repo root on `feat/rust`. Do not delete `app/` or `mcp-server/` yet.
2. Land inbox + CLI + init with tests. Agents can already call `snap latest` if a pair is written by hand.
3. Land the daemon socket and `start` / `stop` / `status`.
4. Land capture + GPUI overlay + save.
5. Point `snap.plist` and `snap.service` at `snap daemon`.
6. When `snap capture` saves a pair and `snap latest` reads it, delete `app/`, `mcp-server/`, `setup-mcp.sh`, and the old Makefile Tauri targets.
7. Rewrite `README.md`, `SETUP.md`, and `CLAUDE.md` for the CLI.
8. Do not copy `~/.snap/inbox/` into a repo. `snap doctor` can mention the old global inbox if it exists.

## 14. Functional requirements
1. The install is one `snap` binary. MUST.
2. `snap daemon` MUST stay up with no window until capture.
3. `snap start` MUST be a no-op when a ping succeeds.
4. `snap capture` and `Ctrl+Shift+S` MUST open one overlay when last-active is valid.
5. Save MUST write a PNG + JSON pair into `<active-repo>/.snap/inbox/`.
6. `snap check`, `latest`, `list`, `get`, and `clear` MUST work when the daemon is down.
7. Those commands MUST resolve the repo by cwd walk, then last-active.
8. `snap init` MUST create `.snap/`, register the repo, set last-active, and patch agent markdown.
9. `snap init` MUST be safe to run twice.
10. The overlay MUST implement the six tools and the keys in §9.
11. Composite MUST include the drawings in the PNG at native capture resolution.
12. Logs MUST rotate at 1 MB and MUST NOT contain image bytes.
13. A second daemon MUST exit if a peer answers ping.

## 15. Acceptance criteria
- [ ] `cargo test` passes with no GPU.
- [ ] `snap init` in a temp git repo creates `.snap/` and an `AGENTS.md` block. A second run does not duplicate the block.
- [ ] After init, `snap latest` exits 1 with the empty-inbox error.
- [ ] A hand-written pair in `.snap/inbox/` is returned by `snap latest` with `image_path` set.
- [ ] `snap check` reports the new pair, then reports none after `latest`.
- [ ] `snap clear` removes the pair and `last_read`.
- [ ] `snap start` then `snap status` shows the pid and last-active path.
- [ ] `snap start` again prints that the daemon is already up and exits 0.
- [ ] `snap capture` (or the hotkey) opens the overlay when last-active is valid.
- [ ] Enter writes a PNG that contains the drawings and a sidecar that matches §8.
- [ ] Escape discards and leaves the inbox unchanged.
- [ ] `snap stop` ends the daemon. Agent commands still work on disk.
- [ ] `snap doctor` names a missing Screen Recording permission on a macOS host that denied it.
- [ ] `app/` and `mcp-server/` are gone after the overlay save works.

## 16. Key decisions

| Decision | Choice | Why |
|----------|--------|-----|
| Stack | one Rust crate, one binary | fewer runtimes, one install asset |
| Agent API | CLI, not MCP | agents already run shell commands |
| UI | GPUI + `tiny-skia` | no WebView; GPU overlay; deterministic export |
| Process | one daemon process | shared last-active; faster overlay; no lock-file overlay child |
| Inbox | per-repo `.snap/inbox/` | agents in repo A do not see repo B |
| Hotkey target | last-active project | a browser has no repo cwd |
| Agent I/O | filesystem, not socket | daemon down does not block `snap latest` |
| Old tree | keep until overlay save works, then delete | safer cutover on `feat/rust` |
| GPUI channel | git pin, not crates.io 0.2.2 | need `gpui_platform::application()` and current window APIs |

## 17. Alternatives (rejected)
1. **Keep Tauri, replace only MCP.** Rejected. The user asked for a Rust core app and GPUI.
2. **Two processes (`snap daemon` headless + `snap overlay` child).** Rejected. More lock files and a slower first paint. Last-active still needs IPC.
3. **Two crates (`snap_core` + `snap`).** Rejected. Extra workspace work for one binary. Revisit if `src/` grows past about 2k lines per module.
4. **Global inbox only.** Rejected. The user chose per-repo.
5. **Pick the repo from the frontmost window.** Rejected. The common case is a browser.
6. **Ask the project on every capture.** Rejected. Slower. The chip cycle covers a miss.

## 18. Open questions
None. The items below are implementation notes, not product questions.
- Pin the GPUI git rev when the crate is added.
- Choose the bundled font file when export lands (any OFL/SIL face that covers ASCII).
- Linux `snap.service` `ExecStart` uses the installed binary path, not `~/Desktop/snap`.
