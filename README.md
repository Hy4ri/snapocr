# snapocr

Select a region of the screen, OCR the text in it, copy to clipboard.

## Usage

```bash
nix develop            # dev shell (rust, tesseract, test rig)
cargo build --release
./target/release/snapocr            # default language: eng
./target/release/snapocr --lang ara # arabic
./target/release/snapocr --debug    # keep the cropped png in /tmp
```

Or install: `nix run .#` (wraps tesseract + runtime libs).

- X11 / Xwayland: native XCB grab.
- Wayland: XDG Desktop Portal screenshot (works on any compositor).
- Drag to select, release to OCR, text lands in the clipboard, auto-exits.
- Esc quits anytime.
