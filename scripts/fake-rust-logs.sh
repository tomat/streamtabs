#!/usr/bin/env bash
set -euo pipefail

# Continuous fake Rust-style logs for local testing.
# Usage:
#   ./scripts/fake-rust-logs.sh
#   ./scripts/fake-rust-logs.sh | ./target/release/streamtabs info warn error debug
# Color control (Rust-style):
#   RUST_LOG_STYLE=always|auto|never
#   RUST_LOG_COLOR=always|auto|never   # alias
#   NO_COLOR=1                         # force disable

rand_from_array() {
  local array_name="$1[@]"
  local arr=("${!array_name}")
  local n="${#arr[@]}"
  printf '%s' "${arr[$((RANDOM % n))]}"
}

hex8() {
  printf '%08x' "$(( (RANDOM << 16) | RANDOM ))"
}

ts() {
  date -u '+%Y-%m-%dT%H:%M:%SZ'
}

sleep_jitter() {
  # ~3x faster than the previous 120-599ms interval.
  local ms=$((40 + RANDOM % 160))
  local s
  printf -v s '0.%03d' "$ms"
  sleep "$s"
}

color_init() {
  local style="${RUST_LOG_STYLE:-${RUST_LOG_COLOR:-auto}}"
  style="$(printf '%s' "$style" | tr '[:upper:]' '[:lower:]')"

  COLOR_ENABLED=0
  if [[ -n "${NO_COLOR:-}" ]]; then
    COLOR_ENABLED=0
  else
    case "$style" in
      always|1|true) COLOR_ENABLED=1 ;;
      never|0|false) COLOR_ENABLED=0 ;;
      auto|"")
        if [[ -t 1 ]]; then
          COLOR_ENABLED=1
        fi
        ;;
      *)
        if [[ -t 1 ]]; then
          COLOR_ENABLED=1
        fi
        ;;
    esac
  fi

  if (( COLOR_ENABLED )); then
    C_RESET=$'\033[0m'
    C_DIM=$'\033[2m'
    C_INFO=$'\033[32m'
    C_DEBUG=$'\033[34m'
    C_WARN=$'\033[33m'
    C_ERROR=$'\033[31m'
  else
    C_RESET=""
    C_DIM=""
    C_INFO=""
    C_DEBUG=""
    C_WARN=""
    C_ERROR=""
  fi
}

level_color() {
  case "$1" in
    INFO) printf '%s' "$C_INFO" ;;
    DEBUG) printf '%s' "$C_DEBUG" ;;
    WARN) printf '%s' "$C_WARN" ;;
    ERROR) printf '%s' "$C_ERROR" ;;
    *) printf '%s' "" ;;
  esac
}

modules_info=(
  "app::startup"
  "api::http"
  "worker::scheduler"
  "db::pool"
  "cache::lru"
  "telemetry::otlp"
)

modules_debug=(
  "runtime::reactor"
  "state::machine"
  "auth::token"
  "api::router"
  "worker::queue"
  "db::query"
)

modules_warn=(
  "db::query"
  "cache::lru"
  "worker::retry"
  "net::client"
  "runtime::memory"
)

modules_error=(
  "api::http"
  "db::pool"
  "worker::executor"
  "net::client"
  "events::publisher"
)

emit_log_line() {
  local roll=$((RANDOM % 100))
  local level
  local module
  local msg
  local req_id
  req_id="$(hex8)"

  # Heavily bias the stream toward DEBUG so it appears very frequently.
  if (( roll < 12 )); then
    level="INFO"
    module="$(rand_from_array modules_info)"
    case $((RANDOM % 5)) in
      0) msg="request finished method=GET path=/v1/accounts status=200 latency_ms=$((8 + RANDOM % 120)) req_id=${req_id}" ;;
      1) msg="worker tick queue_depth=$((RANDOM % 40)) inflight=$((RANDOM % 12))" ;;
      2) msg="cache hit cache=profiles key=user:$((1000 + RANDOM % 9000)) ttl_s=$((20 + RANDOM % 240))" ;;
      3) msg="db query ok sql=select_user rows=$((1 + RANDOM % 3)) elapsed_ms=$((2 + RANDOM % 60))" ;;
      *) msg="flush complete span=metrics batch=$((10 + RANDOM % 200)) elapsed_ms=$((4 + RANDOM % 30))" ;;
    esac
  elif (( roll < 92 )); then
    level="DEBUG"
    module="$(rand_from_array modules_debug)"
    case $((RANDOM % 5)) in
      0) msg="poll cycle reactor_tick=$((10000 + RANDOM % 50000)) wakeups=$((RANDOM % 8))" ;;
      1) msg="state update shard=$((RANDOM % 16)) lease_epoch=$((200 + RANDOM % 50))" ;;
      2) msg="retry scheduled op=fetch_profile backoff_ms=$((20 + RANDOM % 180)) attempt=$((1 + RANDOM % 3))" ;;
      3) msg="span close name=http_request elapsed_ms=$((4 + RANDOM % 90)) req_id=${req_id}" ;;
      *) msg="token refresh check skew_ms=$((RANDOM % 1200)) cached=true" ;;
    esac
  elif (( roll < 98 )); then
    level="WARN"
    module="$(rand_from_array modules_warn)"
    case $((RANDOM % 5)) in
      0) msg="slow query sql=list_sessions elapsed_ms=$((180 + RANDOM % 800)) rows=$((2 + RANDOM % 20))" ;;
      1) msg="cache miss cache=profiles key=user:$((1000 + RANDOM % 9000))" ;;
      2) msg="retrying request attempt=$((2 + RANDOM % 4)) error=timeout backoff_ms=$((80 + RANDOM % 300))" ;;
      3) msg="connection jitter p95_ms=$((50 + RANDOM % 180)) endpoint=payments" ;;
      *) msg="high memory rss_mb=$((500 + RANDOM % 800)) threshold_mb=1024" ;;
    esac
  else
    level="ERROR"
    module="$(rand_from_array modules_error)"
    case $((RANDOM % 5)) in
      0) msg="request failed method=POST path=/v1/checkout status=500 error=database_unavailable req_id=${req_id}" ;;
      1) msg="db connection lost addr=10.0.0.$((2 + RANDOM % 30)):5432 retry_in_ms=$((200 + RANDOM % 900))" ;;
      2) msg="publish failed topic=events.orders error=broken_pipe attempt=$((1 + RANDOM % 2))" ;;
      3) msg="task failed worker_id=$((1 + RANDOM % 8)) job=sync_catalog reason=panic_index_out_of_bounds" ;;
      *) msg="upstream error service=inventory code=UNAVAILABLE retryable=true" ;;
    esac
  fi

  local lvl_color
  lvl_color="$(level_color "$level")"
  printf '%b%s%b %b%-5s%b %b%-20s%b %s\n' \
    "$C_DIM" "$(ts)" "$C_RESET" \
    "$lvl_color" "$level" "$C_RESET" \
    "$C_DIM" "$module" "$C_RESET" \
    "$msg"
}

main() {
  color_init

  local backfill_count=1000
  local i
  for ((i = 0; i < backfill_count; i++)); do
    emit_log_line
  done

  while true; do
    emit_log_line
    sleep_jitter
  done
}

main "$@"
