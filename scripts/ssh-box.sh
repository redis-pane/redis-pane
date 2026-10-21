#!/usr/bin/env bash
# A bastion in a box for checking redis-pane over SSH: sshd and redis-pane in one
# container, with a Redis bound to the container's loopback. Only SSH is
# published, so the one way to the keyspace is to ssh in and run redis-pane
# there — copying (OSC 52), the locale fallback and the timeouts all take their
# SSH paths. See scripts/ssh-box/Dockerfile.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NAME="${REDIS_PANE_SSH_CONTAINER:-redis-pane-ssh}"
IMAGE="${REDIS_PANE_SSH_IMAGE:-redis-pane-ssh}"
PORT="${REDIS_PANE_SSH_PORT:-2222}"
KEY_DIR="$ROOT/target/ssh-box"
KEY="$KEY_DIR/id_ed25519"

usage() {
  cat <<'USAGE'
usage: scripts/ssh-box.sh [up|down|restart|ssh|shell|logs|status]

  up       build the image from this checkout and start the container
  down     stop and remove it
  restart  down, then up (rebuilds, so it picks up code changes)
  ssh      ssh in and run redis-pane; extra arguments go to redis-pane
  shell    ssh in to a shell (fixtures, churn, tmux and redis-cli are there)
  logs     follow sshd's log
  status   print container state and DBSIZE

env:
  REDIS_PANE_SSH_CONTAINER  container name  (default redis-pane-ssh)
  REDIS_PANE_SSH_PORT       host ssh port   (default 2222, on 127.0.0.1 only)
  FIXTURE_KEYS              keys seeded at start (default 5000)

To see the ASCII fallback an empty forwarded locale gets:
  env -u LANG -u LC_ALL -u LC_CTYPE scripts/ssh-box.sh ssh
USAGE
}

running() { [ "$(docker container inspect -f '{{.State.Running}}' "$NAME" 2>/dev/null || echo false)" = true ]; }
exists()  { docker container inspect "$NAME" >/dev/null 2>&1; }

# A throwaway key and no known_hosts entry: the host key changes on every
# rebuild, and this box holds nothing worth protecting.
ssh_to_box() {
  ssh -i "$KEY" -p "$PORT" \
    -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
    -o IdentitiesOnly=yes \
    "$@"
}

up() {
  if [ ! -f "$KEY" ]; then
    mkdir -p "$KEY_DIR"
    ssh-keygen -q -t ed25519 -N '' -C redis-pane-ssh-box -f "$KEY"
  fi
  docker build -t "$IMAGE" -f "$ROOT/scripts/ssh-box/Dockerfile" "$ROOT"
  exists && docker rm -f "$NAME" >/dev/null
  docker run -d --name "$NAME" -p "127.0.0.1:$PORT:22" \
    -e AUTHORIZED_KEY="$(cat "$KEY.pub")" -e FIXTURE_KEYS="${FIXTURE_KEYS:-5000}" \
    "$IMAGE" >/dev/null
  printf 'waiting for sshd'
  for _ in $(seq 1 100); do
    if ssh_to_box -o ConnectTimeout=1 -o BatchMode=yes pane@127.0.0.1 true 2>/dev/null; then
      echo " … ready"
      echo "next: scripts/ssh-box.sh ssh"
      return 0
    fi
    printf .
    sleep 0.3
  done
  echo; echo "sshd did not accept a login in time; see: scripts/ssh-box.sh logs" >&2
  exit 1
}

case "${1:-up}" in
  up)      up ;;
  down)    docker rm -f "$NAME" >/dev/null 2>&1 && echo "removed $NAME" || echo "$NAME not present" ;;
  restart) "$0" down; "$0" up ;;
  # The remote shell re-splits the command line, so each argument is quoted.
  # Guarded because printf with no arguments still prints '' once.
  ssh)
    shift
    args=""
    [ $# -gt 0 ] && args="$(printf '%q ' "$@")"
    ssh_to_box -t pane@127.0.0.1 "redis-pane $args" ;;
  shell)   ssh_to_box -t pane@127.0.0.1 ;;
  logs)    exec docker logs -f "$NAME" ;;
  status)
    if running; then
      echo "$NAME: running, ssh on 127.0.0.1:$PORT"
      docker exec "$NAME" redis-cli DBSIZE
    else
      echo "$NAME: not running"
    fi ;;
  -h|--help|help) usage ;;
  *) usage; exit 2 ;;
esac
