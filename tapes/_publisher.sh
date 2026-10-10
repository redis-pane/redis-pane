#!/bin/sh
# Hidden publisher for the pubsub tape: <delay seconds>, then a bounded stream of messages on a
# channel and on channels a pattern matches. Runs inside redis-pane-demo only; ends by itself.
sleep "${1:-0}"
docker exec redis-pane-demo sh -c '
i=0
for ev in login purchase logout refund login; do
  i=$((i+1))
  redis-cli PUBLISH news "deploy $i finished" >/dev/null
  sleep 0.7
  redis-cli PUBLISH user:100$i:events "{\"event\":\"$ev\",\"user\":100$i,\"ok\":true}" >/dev/null
  sleep 0.7
  redis-cli PUBLISH metrics "ignored: nobody subscribed" >/dev/null
  redis-cli PUBLISH user:100$i:events "{\"event\":\"seen\",\"user\":100$i}" >/dev/null
  sleep 0.7
done'
