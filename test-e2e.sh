#!/usr/bin/env bash
# headless E2E: Xvfb + feh background with known text → drag selection →
# OCR → clipboard must contain the text.
set -euo pipefail
cd "$(dirname "$0")"

convert -size 1280x800 xc:"#f5f5f5" \
  -font DejaVu-Sans -pointsize 64 -fill "#111111" \
  -annotate +120+220 "HELLO SNAPOCR 42" \
  -annotate +120+360 "the quick brown fox" \
  /tmp/snapocr-test-bg.png

export DISPLAY=:99
Xvfb :99 -screen 0 1280x800x24 >/tmp/xvfb.log 2>&1 &
XVFB_PID=$!
trap 'kill $XVFB_PID 2>/dev/null || true' EXIT
sleep 1

feh --bg-center /tmp/snapocr-test-bg.png
sleep 0.5

LIBGL_ALWAYS_SOFTWARE=1 ./target/debug/snapocr &
APP_PID=$!
sleep 2

# drag over the "HELLO SNAPOCR 42" line (region ~120..800 x, 150..300 y)
xdotool mousemove 150 260 mousedown 1 mousemove 800 310 mouseup 1

sleep 5
kill $APP_PID 2>/dev/null || true

CLIP=$(xclip -selection clipboard -o 2>/dev/null || echo "")
echo "clipboard: >>>$CLIP<<<"
if grep -q "HELLO SNAPOCR 42" <<<"$CLIP"; then
  echo "PASS"
else
  echo "FAIL"
  exit 1
fi
