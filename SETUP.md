# Snap Setup Guide

Build, install, and configure the `snap` CLI and daemon.

---

## Prerequisites

### Required

| Dependency | Purpose | Install |
|-----------|---------|---------|
| Rust toolchain | Builds `snap` | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |

You do not need Node, Python, or a WebView.

### macOS compile note

A first compile of GPUI may need the Command Line Tools SDK. If Xcode 26 bindgen fails, point `SDKROOT` at `MacOSX15.4.sdk` or the newest 15.x SDK:

```bash
export SDKROOT="/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk"
export BINDGEN_EXTRA_CLANG_ARGS="--target=arm64-apple-darwin -isysroot ${SDKROOT}"
```

### Linux capture tool

Install **one** of these tools:

| Display Server | Tool | Install |
|---------------|------|---------|
| GNOME on Wayland | `gnome-screenshot` | `sudo apt install gnome-screenshot` |
| wlroots Wayland (Sway, Hyprland) | `grim` | `sudo apt install grim` |
| X11 | `scrot` | `sudo apt install scrot` |

Check your display server:

```bash
echo $XDG_SESSION_TYPE
```

Snap tries capture tools in order and uses the first one that works.

### Window context (optional, X11 only)

```bash
sudo apt install xdotool
```

This fills the sidecar `source` fields. On Wayland those fields stay empty. Annotations still save.

---

## Build and install

```bash
make install
```

`make install` runs `cargo build --release`, copies the binary to `~/.local/bin/snap`, and starts the daemon at login:

- macOS: launchd loads `com.adjective.snap.plist` and runs `snap daemon`
- Linux: systemd user service runs `snap daemon`

Add `~/.local/bin` to your `PATH` if `snap` is not found.

### Service commands

```bash
make start     # snap start
make stop      # snap stop
make status    # snap status
make logs      # tail ~/.snap/snap.log
```

You can also run the same commands on the binary:

```bash
snap start
snap stop
snap status
snap doctor
```

---

## Init a repo

Run this once in each project that should receive snaps:

```bash
cd /path/to/repo
snap init
```

`snap init` creates `.snap/`, registers the repo, sets it as last-active, and patches agent markdown (`AGENTS.md` or `CLAUDE.md`).

A second run is safe. It does not duplicate the markdown block.

To point the hotkey at a different repo:

```bash
snap use /path/to/other/repo
```

---

## macOS permissions

The daemon needs Screen Recording and Accessibility.

1. Start the daemon (`make install` or `snap start`).
2. Press `Ctrl+Shift+S` once.
3. Allow **Screen Recording** if macOS asks.
4. Allow **Accessibility** if macOS asks.

If a prompt never appears, open System Settings > Privacy & Security and add `snap` by hand.

If Screen Recording is missing, `snap doctor` prints:

```
Screen Recording permission is missing. Enable it in System Settings > Privacy & Security > Screen Recording.
```

If permissions get stuck:

```bash
tccutil reset All com.adjective.snap
snap start
```

---

## Verify

```bash
snap doctor
snap status
```

Then:

1. `cd` into an inited repo (or run `snap use` there).
2. Press `Ctrl+Shift+S` (or run `snap capture`).
3. Draw a circle. Press Enter.
4. Run `snap latest`.
5. Confirm `image_path` is under that repo's `.snap/inbox/`.

---

## File Locations

| Path | Purpose |
|------|---------|
| `<repo>/.snap/inbox/` | Annotated screenshots + sidecar JSON |
| `<repo>/.snap/config.toml` | Project name (committed) |
| `<repo>/.snap/last_read` | Agent read cursor (local) |
| `~/.snap/state.toml` | Last-active project |
| `~/.snap/projects.toml` | Registry of inited repos |
| `~/.snap/snap.sock` | Daemon Unix socket |
| `~/.snap/snap.pid` | Daemon pid |
| `~/.snap/snap.log` | Event log (auto-rotates at 1 MB) |

On Linux, if `XDG_RUNTIME_DIR` is set, the socket is `$XDG_RUNTIME_DIR/snap.sock`.

---

## Troubleshooting

### `snap` is not found

Add `~/.local/bin` to `PATH`, or run `~/.local/bin/snap`.

### Daemon is not running

```bash
snap start
snap status
snap doctor
tail ~/.snap/snap.log
```

### Hotkey does nothing

The daemon must be up. A valid last-active project must exist.

```bash
snap start
snap init          # or: snap use /path/to/repo
snap doctor
```

### Screen capture failed

Install a capture tool for your display server (see Prerequisites). On macOS, grant Screen Recording.

### Overlay does not save

Confirm the last-active repo has `.snap/config.toml`. Check `~/.snap/snap.log` for save errors.

### Old `~/.snap/inbox/` still has files

That global inbox is not used. Per-repo inbox is `<repo>/.snap/inbox/`. `snap doctor` prints a note if the old inbox has `snap-*.json` files. Snap does not migrate those files.

### Log file growing too large

The log auto-rotates at 1 MB and keeps one backup at `snap.log.old`.

---

## Uninstall

```bash
snap stop
rm -f ~/.local/bin/snap
```

macOS:

```bash
launchctl unload ~/Library/LaunchAgents/com.adjective.snap.plist 2>/dev/null || true
rm -f ~/Library/LaunchAgents/com.adjective.snap.plist
```

Linux:

```bash
systemctl --user disable --now snap.service
rm -f ~/.config/systemd/user/snap.service
```

Remove Snap data:

```bash
rm -rf ~/.snap
```

Each repo still has a local `.snap/` directory. Delete those by hand if you want them gone.
