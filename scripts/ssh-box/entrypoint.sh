#!/bin/sh
# Start Redis on loopback only, seed it, then run sshd in the foreground.
set -eu

if [ -z "${AUTHORIZED_KEY:-}" ]; then
  echo "AUTHORIZED_KEY is not set; nobody could log in" >&2
  exit 1
fi
install -d -m 700 -o pane -g pane /home/pane/.ssh
printf '%s\n' "$AUTHORIZED_KEY" > /home/pane/.ssh/authorized_keys
chown pane:pane /home/pane/.ssh/authorized_keys
chmod 600 /home/pane/.ssh/authorized_keys

ssh-keygen -A >/dev/null

# --bind 127.0.0.1 is the point of this container: no published port and no
# other interface reaches Redis. Persistence is off, as in redis-up.sh.
redis-server --bind 127.0.0.1 --protected-mode yes --save "" --appendonly no \
  --notify-keyspace-events KEA --daemonize yes --logfile /var/log/redis.log

for _ in $(seq 1 50); do
  redis-cli ping 2>/dev/null | grep -q PONG && break
  sleep 0.2
done
python3 /opt/redis-pane-scripts/fixtures.py --flush -n "${FIXTURE_KEYS:-5000}"

exec /usr/sbin/sshd -D -e
