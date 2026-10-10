#!/usr/bin/env bash
# Render the demo recordings into site/public/demos/.
#
#   ./tapes/render.sh            # every tape
#   ./tapes/render.sh hero       # one (or several): ./tapes/render.sh hero monitor
#
# Safety: this script starts its OWN Redis, the container `redis-pane-demo` on port 6390, and
# removes only that container. It never touches `redis-pane-dev` (6379) or `redis-pane-cluster`.
# XDG_CONFIG_HOME and XDG_STATE_HOME point at a temp dir, so no real Profile or saved state is
# read, and every tape connects with an explicit --url to 6390.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAPES="$ROOT/tapes"
OUT="$ROOT/site/public/demos"
NAME="redis-pane-demo"   # the only container this script may remove
PORT=6390
IMAGE="${REDIS_PANE_IMAGE:-redis:8.4-alpine}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/redis-pane-demo.XXXXXX")"

die() { echo "render.sh: $*" >&2; exit 1; }

cleanup() {
  local rc=$?
  # Writers a tape failed to kill: matched by our port, so nobody else's churn is touched.
  pkill -f "churn.py.*--port $PORT" 2>/dev/null || true
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  rm -rf "$WORK"
  [ "$rc" -eq 0 ] || echo "render.sh: failed (exit $rc)" >&2
}
trap cleanup EXIT

for tool in vhs ffmpeg docker python3 cargo; do
  command -v "$tool" >/dev/null 2>&1 || [ -x "$HOME/.cargo/bin/$tool" ] || die "$tool is required (see tapes/README.md)"
done
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

# Which tapes: arguments, or every tape that is not a partial (leading underscore).
if [ "$#" -gt 0 ]; then
  SCENARIOS=("$@")
else
  SCENARIOS=()
  for f in "$TAPES"/[!_]*.tape; do SCENARIOS+=("$(basename "$f" .tape)"); done
fi
for s in "${SCENARIOS[@]}"; do [ -f "$TAPES/$s.tape" ] || die "no such tape: tapes/$s.tape"; done

echo "==> building target/release/redis-pane"
(cd "$ROOT" && cargo build --release -p redis-pane)

# Temp config and state: no real Profile, no saved session, nothing of the user's.
export XDG_CONFIG_HOME="$WORK/config" XDG_STATE_HOME="$WORK/state" XDG_CACHE_HOME="$WORK/cache"
mkdir -p "$XDG_CONFIG_HOME/redis-pane" "$XDG_STATE_HOME/redis-pane" "$XDG_CACHE_HOME"
unset REDIS_URL REDIS_HOST REDIS_PORT REDIS_USER REDIS_PASSWORD
export PATH="$ROOT/target/release:$PATH"

rcli() { docker exec "$NAME" redis-cli "$@" >/dev/null; }

start_redis() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  docker run -d --name "$NAME" -p "127.0.0.1:$PORT:6379" "$IMAGE" \
    redis-server --save "" --appendonly no >/dev/null
  for _ in $(seq 1 50); do
    [ "$(docker exec "$NAME" redis-cli ping 2>/dev/null || true)" = PONG ] && return 0
    sleep 0.2
  done
  die "$NAME did not come up"
}

# Deterministic data: the seeded fixtures, then the small per-scenario additions.
seed() {
  # The app saves its last filter and scroll under XDG_STATE_HOME; a fresh state per scenario
  # stops one recording's filter from opening the next one.
  rm -rf "$XDG_STATE_HOME" && mkdir -p "$XDG_STATE_HOME/redis-pane"
  python3 "$ROOT/scripts/fixtures.py" --port "$PORT" --flush --seed 7 >/dev/null
  case "$1" in
    hero) rcli HSET demo:live status idle hits 0 ;;
  esac
}

encode() {  # encode <name>: the VHS master (mp4) becomes webm + mp4 + poster (+ gif)
  local n="$1" raw="$WORK/$1.master.mp4" poster
  poster="$(sed -n 's/^# poster: *\([0-9.]*\).*/\1/p' "$TAPES/$n.tape" | head -1)"
  [ -n "$poster" ] || die "tapes/$n.tape has no '# poster: <seconds>' line"
  mv "$OUT/$n.mp4" "$raw"
  ffmpeg -v error -y -i "$raw" -an -c:v libvpx-vp9 -crf 38 -b:v 0 -row-mt 1 -pix_fmt yuv420p "$OUT/$n.webm"
  ffmpeg -v error -y -i "$raw" -an -c:v libx264 -preset slow -crf 30 -pix_fmt yuv420p -movflags +faststart "$OUT/$n.mp4"
  ffmpeg -v error -y -ss "$poster" -i "$raw" -frames:v 1 "$OUT/$n.png"
  if grep -q '^# gif: *yes' "$TAPES/$n.tape"; then
    ffmpeg -v error -y -i "$raw" -vf "fps=8,scale=720:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=32:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" "$OUT/$n.gif"
  else
    rm -f "$OUT/$n.gif"
  fi
}

mkdir -p "$OUT"
echo "==> starting $NAME on :$PORT"
start_redis

for s in "${SCENARIOS[@]}"; do
  echo "==> $s"
  seed "$s"
  # Run from tapes/ so `Source _settings.tape` and the relative Output paths resolve.
  # The tape writes its master as $OUT/$s.mp4; encode() replaces it.
  (cd "$TAPES" && vhs "$s.tape")
  pkill -f "churn.py.*--port $PORT" 2>/dev/null || true
  encode "$s"
done

echo "==> done"
ls -l "$OUT"
