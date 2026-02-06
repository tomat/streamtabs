#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$ROOT_DIR/docs/screenshots"
APP="${STREAMTABS_SCREENSHOT_APP:-Terminal}"
COLS="${STREAMTABS_SCREENSHOT_COLS:-120}"
ROWS="${STREAMTABS_SCREENSHOT_ROWS:-30}"
BOOT_DELAY="${STREAMTABS_SCREENSHOT_BOOT_DELAY:-24}"
STATE_DELAY="${STREAMTABS_SCREENSHOT_STATE_DELAY:-1}"
SELECT_DELAY="${STREAMTABS_SCREENSHOT_SELECT_DELAY:-0.25}"
LIVE_WEBP="${STREAMTABS_SCREENSHOT_LIVE_WEBP:-1}"
LIVE_WEBP_FRAMES="${STREAMTABS_SCREENSHOT_LIVE_WEBP_FRAMES:-18}"
LIVE_WEBP_FRAME_DELAY="${STREAMTABS_SCREENSHOT_LIVE_WEBP_FRAME_DELAY:-0.12}"
WINDOW_ID=""
FRAME_DIR=""
WINDOW_LEFT="${STREAMTABS_SCREENSHOT_WINDOW_LEFT:-80}"
WINDOW_TOP="${STREAMTABS_SCREENSHOT_WINDOW_TOP:-60}"
WINDOW_RIGHT="${STREAMTABS_SCREENSHOT_WINDOW_RIGHT:-1480}"
WINDOW_BOTTOM="${STREAMTABS_SCREENSHOT_WINDOW_BOTTOM:-980}"
WINDOW_TITLE="${STREAMTABS_SCREENSHOT_WINDOW_TITLE:-}"
WINDOW_TITLE_PAD="${STREAMTABS_SCREENSHOT_WINDOW_TITLE_PAD:-220}"
SELECT_KEY="${STREAMTABS_SCREENSHOT_SELECT_KEY:-s}"
ERROR_TAB_KEY="${STREAMTABS_SCREENSHOT_ERROR_TAB_KEY:-1}"
CONTEXT_TAB_KEY="${STREAMTABS_SCREENSHOT_CONTEXT_TAB_KEY:-4}"
printf -v WINDOW_TITLE_PADDED "%s%*s" "$WINDOW_TITLE" "$WINDOW_TITLE_PAD" ""

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

cleanup() {
  if [[ -n "${FRAME_DIR}" && -d "${FRAME_DIR}" ]]; then
    rm -rf "${FRAME_DIR}"
  fi

  if [[ -n "${WINDOW_ID}" ]]; then
    osascript <<OSA >/dev/null 2>&1 || true
tell application "$APP"
  repeat with w in windows
    if (id of w) is ${WINDOW_ID} then
      close w saving no
      exit repeat
    end if
  end repeat
end tell
OSA
  fi
}
trap cleanup EXIT

send_key() {
  local key="$1"
  osascript <<OSA >/dev/null
tell application "$APP" to activate
tell application "System Events"
  keystroke "$key"
end tell
OSA
}

refresh_title() {
  osascript <<OSA >/dev/null
tell application "$APP"
  try
    set title displays custom title of front window to false
  end try
  try
    set title displays shell path of front window to false
  end try
  try
    set title displays device name of front window to false
  end try
  try
    set title displays window size of front window to false
  end try
  try
    set title displays settings name of front window to false
  end try
  try
    set custom title of front window to ""
  end try
  try
    set custom title of selected tab of front window to "$WINDOW_TITLE_PADDED"
  end try
  try
    set title displays custom title of selected tab of front window to true
  end try
  try
    set title displays shell path of selected tab of front window to false
  end try
  try
    set title displays device name of selected tab of front window to false
  end try
  try
    set title displays window size of selected tab of front window to false
  end try
  try
    set title displays file name of selected tab of front window to false
  end try
end tell
OSA
}

require_cmd osascript
require_cmd screencapture

mkdir -p "$OUT_DIR"

cd "$ROOT_DIR"

if [[ ! -x "$ROOT_DIR/target/release/streamtabs" ]]; then
  echo "Missing release binary: $ROOT_DIR/target/release/streamtabs" >&2
  echo "Build it first with: cargo build --release" >&2
  exit 1
fi

if [[ "$LIVE_WEBP" == "1" ]]; then
  require_cmd magick
fi

WINDOW_ID="$(osascript <<OSA
tell application "$APP"
  activate
  do script "cd '$ROOT_DIR'; RUST_LOG_COLOR=always ./scripts/fake-rust-logs.sh | ./target/release/streamtabs ERROR WARN INFO DEBUG"
  set bounds of front window to {$WINDOW_LEFT, $WINDOW_TOP, $WINDOW_RIGHT, $WINDOW_BOTTOM}
  try
    set title displays custom title of front window to false
  end try
  try
    set title displays shell path of front window to false
  end try
  try
    set title displays device name of front window to false
  end try
  try
    set title displays window size of front window to false
  end try
  try
    set title displays settings name of front window to false
  end try
  try
    set custom title of front window to ""
  end try
  try
    set custom title of selected tab of front window to "$WINDOW_TITLE_PADDED"
  end try
  try
    set title displays custom title of settings set of front window to true
    set title displays shell path of settings set of front window to false
    set title displays device name of settings set of front window to false
    set title displays window size of settings set of front window to false
    set title displays settings name of settings set of front window to false
  end try
  try
    set title displays custom title of selected tab of front window to true
    set title displays shell path of selected tab of front window to false
    set title displays device name of selected tab of front window to false
    set title displays window size of selected tab of front window to false
    set title displays file name of selected tab of front window to false
  end try
  try
    set number of columns of front window to $COLS
    set number of rows of front window to $ROWS
  end try
end tell
delay 0.8
try
  tell application "System Events"
    tell process "$APP"
      return value of attribute "AXWindowNumber" of front window
    end tell
  end tell
on error
  tell application "$APP"
    return id of front window
  end tell
end try
OSA
)"
WINDOW_ID="$(printf '%s' "$WINDOW_ID" | tr -dc '0-9')"

if [[ -z "$WINDOW_ID" ]]; then
  echo "Failed to get $APP front window id for screenshot capture." >&2
  exit 1
fi

sleep "$BOOT_DELAY"
refresh_title

if [[ "$LIVE_WEBP" == "1" ]]; then
  FRAME_DIR="$(mktemp -d "${TMPDIR:-/tmp}/streamtabs-live-frames.XXXXXX")"

  for ((i = 0; i < LIVE_WEBP_FRAMES; i++)); do
    printf -v frame_num '%03d' "$i"
    frame_path="$FRAME_DIR/live-$frame_num.png"
    screencapture -x -l "$WINDOW_ID" "$frame_path"
    sleep "$LIVE_WEBP_FRAME_DELAY"
  done

  WEBP_DELAY_CENTIS="$(awk -v delay="$LIVE_WEBP_FRAME_DELAY" 'BEGIN { cs = int((delay * 100) + 0.5); if (cs < 1) cs = 1; print cs }')"
  magick "$FRAME_DIR"/live-*.png \
    -set delay "$WEBP_DELAY_CENTIS" \
    -loop 0 \
    -quality 90 \
    -define webp:method=6 \
    "$OUT_DIR/live.webp"
else
  :
fi

send_key "3"
sleep "$STATE_DELAY"
refresh_title
screencapture -x -l "$WINDOW_ID" "$OUT_DIR/filtered.png"

send_key "$ERROR_TAB_KEY"
sleep "$STATE_DELAY"
send_key "$SELECT_KEY"
sleep "$SELECT_DELAY"
refresh_title
screencapture -x -l "$WINDOW_ID" "$OUT_DIR/selected.png"

send_key " "
sleep "$STATE_DELAY"
send_key "$CONTEXT_TAB_KEY"
sleep "$STATE_DELAY"
refresh_title
screencapture -x -l "$WINDOW_ID" "$OUT_DIR/selected-paused-switched.png"

send_key "q"
sleep 0.5

echo "Updated screenshots:"
if [[ "$LIVE_WEBP" == "1" ]]; then
  echo "  $OUT_DIR/live.webp"
fi
echo "  $OUT_DIR/filtered.png"
echo "  $OUT_DIR/selected.png"
echo "  $OUT_DIR/selected-paused-switched.png"
