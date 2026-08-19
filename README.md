# Snap

**Visual annotation layer for agentic coding.**

Annotate your screen and feed marked-up screenshots to a coding agent. See a bug? Circle it, label it, hit Enter. The agent sees what you marked.

There is no MCP server. Agents run `snap` commands on disk.

```
You see a bug  -->  Ctrl+Shift+S  -->  Circle it, type "fix this"  -->  Enter
                                                                          |
                                                    <repo>/.snap/inbox/
                                                 snap-20260818-142015-897.png
                                                 snap-20260818-142015-897.json
                                                                          |
                                              Agent runs snap latest  -->  Fix applied
```

---

## How It Works

Snap is one Rust binary named `snap`. That binary is the daemon, the agent CLI, the project init tool, and the overlay.

1. Run `snap init` in a repo. This creates `.snap/` and sets the last-active project.
2. Run `snap start` (or `make install`) so the daemon stays up.
3. Press `Ctrl+Shift+S` from any app.
4. Draw on the frozen screen. Hit Enter.
5. Snap writes a PNG + JSON pair to the last-active repo's `.snap/inbox/`.
6. The agent runs `snap latest` and reads the PNG at `image_path`.

The frontmost window does not pick the repo. A browser has no repo cwd. The last-active project selects the inbox.

### Last-active project

`snap init` and `snap use` write `~/.snap/state.toml`. The overlay chip shows that project name. A click on the chip cycles registered repos.

### Per-repo inbox

Each repo owns its inbox at `<repo>/.snap/inbox/`. Snap does not write `~/.snap/inbox/`. Agent commands walk from the cwd to find `.snap/config.toml`. If they find none, they use the last-active project.

### What Gets Saved

Every save produces a matched pair:

**`snap-{timestamp}.png`** — The capture with annotations composited into the image.

**`snap-{timestamp}.json`** — Structured metadata:

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
    },
    {
      "type": "text",
      "position": [350, 310],
      "content": "fix this alignment",
      "color": "#FF3B30"
    }
  ],
  "image_filename": "snap-20260818-142015-897.png"
}
```

`snap latest` adds `filename` and `image_path`. The agent reads the PNG from `image_path`.

---

## Annotation Tools

| Tool | Shortcut | Description |
|------|----------|-------------|
| Circle | `C` | Click-drag to draw ellipses. Shift constrains to a circle. |
| Rectangle | `R` | Click-drag to draw boxes. Shift constrains to a square. 10% fill for visibility. |
| Arrow | `A` | Click start, drag to end. Arrowhead on the endpoint. |
| Freehand | `F` | Click-drag to draw a path. |
| Text | `T` | Click to place a label. Type your instruction. Enter to confirm. |
| Numbered Marker | `N` | Click to place auto-incrementing circled numbers (1, 2, 3...). |

### Controls

| Key | Action |
|-----|--------|
| `Ctrl+Shift+S` | Open the annotation overlay (global, works from any app) |
| `D` | Toggle the dim layer (darkens the background for contrast) |
| `Ctrl+Z` / `Cmd+Z` | Undo the last annotation |
| `Enter` | Save the annotated screenshot and close |
| `Escape` | Cancel the in-progress draw, or discard and close |

### Toolbar

- **6 color swatches**: Red (default), Blue, Green, Yellow, White, Black
- **3 stroke widths**: Thin (2px), Medium (4px, default), Thick (6px)
- **Draggable**: Grab the handle on the left to reposition the toolbar
- **Dim toggle**: Adds a dark overlay behind annotations
- **Project chip**: Shows the last-active repo. Click to cycle registered projects

---

## Quick Start

```bash
# Install the Rust toolchain (once)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build snap and start the daemon at login
make install

# In each repo that should receive snaps
snap init
```

Press `Ctrl+Shift+S`, draw on your screen, hit Enter. Then tell your agent: *"check my latest snap annotation"*

For detailed setup, see **[SETUP.md](SETUP.md)**.

---

## Example Workflow

You open a web app, scroll through, and spot three issues.

**1. Snap the bugs** (takes ~15 seconds)

```
Ctrl+Shift+S  →  circle the broken button, type "this should be blue"  →  Enter
Ctrl+Shift+S  →  arrow at the spacing gap, type "16px here"            →  Enter
Ctrl+Shift+S  →  number markers 1, 2, 3 on misaligned cards            →  Enter
```

Each save drops a PNG+JSON pair into `<repo>/.snap/inbox/`.

**2. Tell your agent** (one prompt)

```
> fix the frontend issues I just annotated
```

If the repo has the Snap block from `snap init`, the agent runs `snap check` at the start of a UI task. It then runs `snap latest` or `snap list` and reads each PNG.

Without that block, say:

```
> run snap latest and fix what I marked
```

**3. Agent reads and fixes**

`snap list 3` returns three records. Each record has `image_path` and structured annotations. The agent reads each image and applies the fixes.

**4. Clear the inbox**

After you accept the work:

```
> looks good, run snap clear
```

The inbox is a conveyor belt, not a filing cabinet. Annotations go in. The agent processes them. You clear it.

---

## Agent Commands

Agents talk to Snap through the CLI. These commands read the inbox on disk. They do not need the daemon.

| Command | Description |
|---------|-------------|
| `snap check` | Are there new annotations since the last successful `latest`/`list`? |
| `snap latest` | Get the most recent annotation. Writes `last_read`. |
| `snap list [n]` | List the N most recent annotations (default 5). Writes `last_read`. |
| `snap get <id>` | Get one annotation by filename stem. Does not write `last_read`. |
| `snap clear` | Delete all pairs in this inbox and remove `last_read`. |

### Lifecycle and project

| Command | Description |
|---------|-------------|
| `snap daemon` | This process is the daemon (GPUI event loop) |
| `snap start` | Start the daemon if a ping fails |
| `snap stop` | Ask the daemon to shut down |
| `snap status` | Daemon up, socket, last-active project |
| `snap doctor` | Permissions, capture tool, socket, last-active path |
| `snap init` | Create `.snap/`, register the repo, patch agent markdown |
| `snap use [path]` | Set the last-active project (default: this repo) |
| `snap projects` | List registered repos and mark the active one |
| `snap capture` | Ask the daemon to open the overlay |

Agent commands print JSON on stdout. Exit 0 means success. Exit 1 means a user or environment error. Exit 2 means a usage error.

### How Agents Interpret Annotations

- **Red circles/rectangles** = "this area has a problem"
- **Arrows** = "this should move/connect there"
- **Text labels** = literal instructions ("fix this", "16px gap", "wrong color")
- **Numbered markers** = ordered list of issues (fix #1 first, then #2, etc.)
- **Freehand marks** = emphasis, "look at this general area"

---

## Architecture

```
snap/
  src/                      One crate, one binary
    main.rs                 clap dispatch
    cli.rs                  Subcommands
    daemon.rs               Socket thread + GPUI run
    overlay.rs              Overlay, toolbar, hotkey, save
    inbox.rs                Per-repo inbox
    project.rs              init, use, resolve
    draw.rs                 Tools + tiny-skia composite
    capture.rs              macOS/Linux screen capture
    window.rs               Window context
    doctor.rs               snap doctor
  assets/                   Bundled export font
  snap.plist                launchd: snap daemon
  snap.service              systemd: snap daemon
  Makefile                  cargo build --release + install
```

### Platform Support

| Platform | Screen Capture | Window Context | Global Hotkey |
|----------|---------------|----------------|---------------|
| macOS | `screencapture` (interactive picker) | AppleScript (title, URL, PID) | Daemon (`Ctrl+Shift+S`) |
| Linux X11 | `scrot` | `xdotool` (title, class, PID) | Daemon (`Ctrl+Shift+S`) |
| Linux Wayland (GNOME) | `gnome-screenshot` | Not available | Daemon (`Ctrl+Shift+S`) |
| Linux Wayland (wlroots) | `grim` | Not available | Daemon (`Ctrl+Shift+S`) |
| Windows | Out of v1 | — | — |

On Linux, Snap tries `gnome-screenshot`, then `grim`, then `scrot`.

---

## Data & Privacy

- All data stays local. Screenshots go to `<repo>/.snap/inbox/` and nowhere else.
- There is no MCP server and no network call. No telemetry. No cloud.
- The log file is `~/.snap/snap.log`. It auto-rotates at 1 MB. It stores event names and paths, not image bytes.

---

## Built By

[Rob Murtha — Adjective LLC](https://adjective.us)

### Contributors

- Alec Lucas — macOS port
- TangoKiloA — Windows support (screen capture, window context, tray)

## License

MIT — see [LICENSE](LICENSE).
