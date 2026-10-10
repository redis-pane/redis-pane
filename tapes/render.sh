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
OUT="$ROOT/site/public/demos"      # webm, mp4, png: what the site serves
GIFS="$ROOT/docs/demos"            # README-only gifs; outside site/, so the static export omits them
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
rpipe() { docker exec -i "$NAME" redis-cli >/dev/null; }   # commands on stdin, one per line

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
  rm -f "$XDG_CONFIG_HOME/redis-pane/config.json"
  python3 "$ROOT/scripts/fixtures.py" --port "$PORT" --flush --seed 7 >/dev/null
  # Slowlog threshold back to the default; only the slowlog scenario lowers it, and only on this
  # container.
  rcli CONFIG SET slowlog-log-slower-than 10000
  rcli SLOWLOG RESET
  case "$1" in
    hero) rcli HSET demo:live status idle hits 0 ;;
    filter)
      # Keys for the `user:*:session` glob: sessions among other per-user keys.
      for id in 1001 1002 1003 1004 1005 1006; do
        rpipe <<EOF2
SET user:$id:session "tok_$id" EX 3600
SET user:$id:profile "{\"id\":$id}"
SET user:$id:cart "empty"
EOF2
      done ;;
    types)
      rpipe <<'EOF2'
SET demo:t:json "{\"id\":4821,\"plan\":\"pro\",\"seats\":12,\"tags\":[\"alpha\",\"orion\"],\"active\":true}"
HSET demo:t:hash name "Ada Lovelace" role admin region eu-west-1 plan pro
RPUSH demo:t:list "resize avatar" "send welcome email" "index document"
SADD demo:t:set orion vega rigel atlas
ZADD demo:t:zset 9120.5 player:41 8044 player:7 7310.25 player:93
XADD demo:t:stream * event login user 41 amount 0
XADD demo:t:stream * event purchase user 7 amount 19.99
XADD demo:t:stream * event refund user 7 amount 19.99
SET demo:t:binary "\x00\x01\xfe\xffPNG\r\n\x1a\n\x89redis-pane"
EOF2
      ;;
    live)
      rcli HSET demo:live status idle hits 0 owner worker-3
      rcli EXPIRE demo:live 55 ;;
    edit-value)
      rcli SET demo:config '{"feature:alpha":true,"feature:orion":false,"version":12,"owner":"platform"}' ;;
    edit-members)
      rpipe <<'EOF2'
HSET demo:m:hash name "Ada Lovelace" role admin region eu-west-1
ZADD demo:m:zset 9120 player:41 8044 player:7 7310 player:93
RPUSH demo:m:list "resize avatar" "send welcome email" "index document" "purge cache"
EOF2
      ;;
    safety|bulk-delete)
      cat > "$XDG_CONFIG_HOME/redis-pane/config.json" <<EOF2
{
  "profiles": {
    "prod": { "host": "127.0.0.1", "port": $PORT, "env": "prod", "note": "demo only" }
  }
}
EOF2
      chmod 600 "$XDG_CONFIG_HOME/redis-pane/config.json"
      rcli SET demo:safe:token "tok_live_9f3a" ;;
    dashboard) ;;
    pubsub) ;;
    slowlog)
      # Threshold 100us on this container only (never 0: the app's own per-row TYPE / TTL /
      # MEMORY USAGE would fill the 128-entry log and push these out). Lua busy loops of
      # different lengths are the slow commands; the app's own SCAN pages add a few more.
      rcli CONFIG SET slowlog-log-slower-than 100
      rcli EVAL "for i=1,6e7 do end return 1" 0
      rcli SET report:daily "ready"
      rcli EVAL "for i=1,2e7 do end return 1" 0
      rcli EVAL "for i=1,5e6 do end return 1" 0
      rcli KEYS 'session:*'
      rcli EVAL "for i=1,4e7 do end return 1" 0
      rcli EVAL "for i=1,1e6 do end return 1" 0
      rcli SORT leaderboard:1 BY nosort
      rcli EVAL "for i=1,8e6 do end return 1" 0 ;;
  esac
  if [ "$1" = bulk-delete ]; then
    # ~2000 keys under one prefix; Redis can write them in one pass.
    python3 - "$PORT" <<'EOF2' | docker exec -i "$NAME" redis-cli >/dev/null
import sys
for i in range(1, 2001):
    print('SET tmp:bulk:%04d "x"' % i)
EOF2
  fi
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
    ffmpeg -v error -y -i "$raw" -vf "fps=8,scale=720:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=32:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" "$GIFS/$n.gif"
  else
    rm -f "$GIFS/$n.gif" "$OUT/$n.gif"
  fi
}

mkdir -p "$OUT" "$GIFS"
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
ls -l "$OUT" "$GIFS"
