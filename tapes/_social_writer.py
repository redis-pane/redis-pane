#!/usr/bin/env python3
"""Hidden writer for the social tape: deterministic, bounded, ends by itself.

Two jobs, both on the demo container (pass --port 6390):

  1. Every 0.7 s, bump `hits` and step `status` (idle -> busy -> done) on the hash `demo:live`.
     That is the one obvious value that changes with no keypress.
  2. From --traffic-after seconds, a steady stream of mixed commands on a fixed set of
     `demo:ops:*` keys (no deletes, and only 40 keys that exist after the first pass, so the
     keyspace the earlier scenes show stays put), with the rate alternating 12 and 40 op/s every
     2 s so Ops/sec visibly moves.

Uses scripts/resp.py only. Stops after --duration seconds or on SIGTERM/SIGINT.
"""
import argparse
import os
import signal
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "scripts"))
from resp import add_target_args, connect  # noqa: E402

running = True


def stop(*_):
    global running
    running = False


def main():
    p = argparse.ArgumentParser()
    add_target_args(p)
    p.add_argument("--start-after", type=float, default=0.0, help="seconds before the first hits bump")
    p.add_argument("--traffic-after", type=float, default=30.0, help="seconds before the background traffic starts")
    p.add_argument("--duration", type=float, default=80.0)
    args = p.parse_args()
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)

    r = connect(args)
    t0 = time.monotonic()
    states = ["idle", "busy", "done"]
    hits = 0
    next_live = args.start_after
    next_op = args.traffic_after
    op = 0
    while running:
        now = time.monotonic() - t0
        if now >= args.duration:
            break
        if now >= next_live:
            hits += 1
            r.call("HSET", "demo:live", "hits", hits, "status", states[hits % 3])
            next_live += 0.7
        if now >= next_op:
            op += 1
            k = "demo:ops:%d" % (op % 8)
            cmd = op % 5
            if cmd == 0:
                r.call("INCR", k + ":count")
            elif cmd == 1:
                r.call("SET", k + ":last", "req-%d" % op)
            elif cmd == 2:
                r.call("HSET", k + ":stats", "ok", op, "ms", 3 + op % 17)
            elif cmd == 3:
                r.call("LPUSH", k + ":queue", "job-%d" % op)
                r.call("LTRIM", k + ":queue", 0, 9)
            else:
                r.call("ZINCRBY", k + ":score", 1, "user:%d" % (op % 5))
            rate = 40.0 if int((now - args.traffic_after) // 2) % 2 == 0 else 12.0
            next_op += 1.0 / rate
        time.sleep(0.005)
    r.close()


if __name__ == "__main__":
    main()
