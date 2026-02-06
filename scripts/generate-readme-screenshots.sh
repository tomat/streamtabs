#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$ROOT_DIR/docs/screenshots"
APP="${STREAMTABS_SCREENSHOT_APP:-Terminal}"
COLS="${STREAMTABS_SCREENSHOT_COLS:-120}"
ROWS="${STREAMTABS_SCREENSHOT_ROWS:-30}"
BOOT_DELAY="${STREAMTABS_SCREENSHOT_BOOT_DELAY:-24}"
STATE_DELAY="${STREAMTABS_SCREENSHOT_STATE_DELAY:-1}"
WINDOW_ID=""
WINDOW_LEFT="${STREAMTABS_SCREENSHOT_WINDOW_LEFT:-80}"
WINDOW_TOP="${STREAMTABS_SCREENSHOT_WINDOW_TOP:-60}"
WINDOW_RIGHT="${STREAMTABS_SCREENSHOT_WINDOW_RIGHT:-1480}"
WINDOW_BOTTOM="${STREAMTABS_SCREENSHOT_WINDOW_BOTTOM:-980}"
WINDOW_TITLE="${STREAMTABS_SCREENSHOT_WINDOW_TITLE:-streamtabs}"
LINE_CLICK_X="${STREAMTABS_SCREENSHOT_LINE_CLICK_X:-$((WINDOW_LEFT + 360))}"
LINE_CLICK_Y="${STREAMTABS_SCREENSHOT_LINE_CLICK_Y:-$((WINDOW_BOTTOM - 90))}"
SWITCH_TAB_KEY="${STREAMTABS_SCREENSHOT_SWITCH_TAB_KEY:-0}"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

cleanup() {
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

click_at() {
  local x="$1"
  local y="$2"
  osascript <<OSA >/dev/null
tell application "$APP" to activate
tell application "System Events"
  click at {$x, $y}
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

WINDOW_ID="$(osascript <<OSA
tell application "$APP"
  activate
  do script "cd '$ROOT_DIR'; RUST_LOG_COLOR=always ./scripts/fake-rust-logs.sh | ./target/release/streamtabs ERROR WARN INFO DEBUG"
  set bounds of front window to {$WINDOW_LEFT, $WINDOW_TOP, $WINDOW_RIGHT, $WINDOW_BOTTOM}
  try
    set custom title of front window to "$WINDOW_TITLE"
  end try
  try
    set custom title of selected tab of front window to "$WINDOW_TITLE"
  end try
  try
    set title displays custom title of front window to true
    set title displays shell path of front window to false
    set title displays device name of front window to false
    set title displays window size of front window to false
    set title displays settings name of front window to false
  end try
  try
    set title displays custom title of settings set of front window to true
    set title displays shell path of settings set of front window to false
    set title displays device name of settings set of front window to false
    set title displays window size of settings set of front window to false
    set title displays settings name of settings set of front window to false
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
screencapture -x -l "$WINDOW_ID" "$OUT_DIR/live.png"

send_key "3"
sleep "$STATE_DELAY"
screencapture -x -l "$WINDOW_ID" "$OUT_DIR/filtered.png"

click_at "$LINE_CLICK_X" "$LINE_CLICK_Y"
sleep "$STATE_DELAY"
screencapture -x -l "$WINDOW_ID" "$OUT_DIR/selected.png"

send_key "$SWITCH_TAB_KEY"
sleep "$STATE_DELAY"
send_key " "
sleep "$STATE_DELAY"
screencapture -x -l "$WINDOW_ID" "$OUT_DIR/selected-paused-switched.png"

send_key "q"
sleep 0.5

echo "Updated screenshots:"
echo "  $OUT_DIR/live.png"
echo "  $OUT_DIR/filtered.png"
echo "  $OUT_DIR/selected.png"
echo "  $OUT_DIR/selected-paused-switched.png"
