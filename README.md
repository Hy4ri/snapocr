# snapocr

A fast, self-contained screen OCR utility for Linux desktops (Wayland and X11).

Select any region on your screen, extract text (English, Arabic, code, symbols) using an optimized in-memory Tesseract pipeline, and copy the result directly to your clipboard with a desktop notification.

Zero external CLI dependencies required (no `slurp`, no `grim`).

---

## Features

- **Self-Contained:** Native in-memory Wayland screencopy via `libwayshot` and interactive freeze-frame selector.
- **Multi-Monitor Support:** Automatically detects and captures the active/focused monitor on multi-display setups (Hyprland and Sway IPC), preventing scaling distortion or window displacement.
- **Auto Multi-Language OCR:** Detects and extracts Arabic and English text simultaneously (`eng+ara`) out-of-the-box.
- **High-Accuracy OCR Pipeline:**
  - 2.5x Catmull-Rom upscaling to match 300 DPI neural OCR requirements.
  - 24px white padding to preserve character connectors and glyph contours.
  - Automatic dark-mode background detection and inversion.
  - Intelligent Page Segmentation Mode (PSM) fallback (PSM 13 for single-line UI titles/snippets, PSM 6 for multiline blocks).
- **Clipboard and Alerts:** Copies to clipboard (`arboard` / `wl-copy`) and delivers a native desktop notification preview (`notify-send`).
- **Nix Flake First-Class Support:** Zero-friction build and dev environment with pinned dependencies.

---

## Usage

```bash
# Default: interactive crop on active monitor with automatic language detection (English + Arabic)
snapocr

# Target a specific monitor
snapocr --monitor DP-1
snapocr --monitor 1

# Capture all monitors combined into a single canvas
snapocr --all

# List detected outputs
snapocr --list-monitors

# Force a specific language
snapocr --lang eng
snapocr --lang ara

# Save preprocessed crop to /tmp for debugging
snapocr --debug

# Suppress desktop notification
snapocr --no-notify
```

---

## Hyprland / Sway Keybind

Add this to your `hyprland.conf`:

```ini
# OCR region to clipboard
bind = $mainMod SHIFT, S, exec, snapocr
```

Or in `sway/config`:

```ini
bindsym $mod+Shift+s exec snapocr
```

---

## Building and Installation

### With Nix Flakes (Recommended)

```bash
# Build the standalone wrapped binary
nix build .#

# Run directly
./result/bin/snapocr

# Enter development shell
nix develop
```

### With Cargo

```bash
cargo build --release
```

---

## License

MIT
