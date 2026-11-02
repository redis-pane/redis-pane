#!/usr/bin/env python3
"""Fill a dev Redis with a keyspace worth browsing.

Every Redis type the Viewers care about is represented, key names are
namespaced so the filter has something to bite on, sizes vary by two orders of
magnitude so the memory column is not a flat line, and a share of the keys
carry a TTL between 10s and 2m so the countdown visibly moves while you watch.

  python3 scripts/fixtures.py                 # 5000 keys, 40% with a TTL
  python3 scripts/fixtures.py -n 200000       # a keyspace big enough to scan
  python3 scripts/fixtures.py --ttl-fraction 1 --flush
"""

import argparse
import json
import random
import string
import sys
import time

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from resp import add_target_args, connect  # noqa: E402

TTL_MIN_SECONDS = 10
TTL_MAX_SECONDS = 120

WORDS = (
    "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda sigma orion "
    "vega rigel atlas hermes juno pluto vesta ceres pallas europa titan rhea"
).split()

NAMESPACES = [
    ("session", "string"),
    ("cache:page", "string"),
    ("cache:user", "hash"),
    ("user", "hash"),
    ("queue", "list"),
    ("feed", "list"),
    ("tag", "set"),
    ("online", "set"),
    ("leaderboard", "zset"),
    ("rank:daily", "zset"),
    ("events", "stream"),
    ("audit", "stream"),
    ("counter", "string"),
    ("flag", "string"),
    ("lock", "string"),
    ("profile", "json"),
    ("config", "json"),
]


def blob(rng, n):
    return "".join(rng.choice(string.ascii_letters + string.digits) for _ in range(n))


def sentence(rng, n=6):
    return " ".join(rng.choice(WORDS) for _ in range(n))


def json_object(rng, key):
    """A plausible JSON-as-string payload (R3.2): still a plain STRING on the
    server, but readable as JSON to redis-pane's own string_value detection
    and, once shipped, its String-edit round trip (PLAN M2 task 4)."""
    if key.startswith("config:"):
        obj = {"feature:%s" % w: rng.random() < 0.5 for w in rng.sample(WORDS, k=min(5, len(WORDS)))}
        obj["version"] = rng.randint(1, 50)
        obj["updated_at"] = int(time.time())
        return obj
    return {
        "id": rng.randint(1, 999_999),
        "plan": rng.choice(["free", "pro", "enterprise"]),
        "seats": rng.randint(1, 50),
        "region": rng.choice(["us-east-1", "eu-west-1", "ap-south-1"]),
        "tags": [rng.choice(WORDS) for _ in range(rng.randint(0, 4))],
        "active": rng.random() < 0.8,
    }


def json_samples():
    """Fixed JSON-as-string keys for eyeballing the JSON viewer (#144): its
    colours, the word-wrap of long strings, the cut of long unbroken tokens,
    scrolling a large document, and the plain-text fallback for JSON that does
    not parse. Built without the RNG, so a seeded keyspace is unchanged."""
    order = {
        "id": 8812,
        "status": "shipped",
        "paid": True,
        "discount": None,
        "total": 129.95,
        "weight_kg": -0.5e-3,
        "items": [
            {"sku": "A-100", "qty": 2, "price": 19.5, "gift": False},
            {"sku": "B-220", "qty": 1, "price": 90.95, "gift": True},
        ],
        "customer": {
            "name": "Ada Lovelace",
            "email": "ada@example.com",
            "vip": False,
            "address": {"city": "Strasbourg", "zip": "67000", "lines": ["1 rue du Dôme", "3e étage"]},
        },
        "note": (
            "Leave the parcel at the reception desk on the ground floor if nobody answers the door, "
            "and call the customer before 6pm so they know it has arrived. The \"back entrance\" "
            "is closed on Sundays."
        ),
        "token": (
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiI4ODEyIiwicm9sZSI6ImFkbWluIiwiZXhwIjoxOTk5"
            "OTk5OTk5fQ.c2lnbmF0dXJlLW9mLWEtdmVyeS1sb25nLWp3dC10b2tlbi1mb3ItdGVzdGluZw"
        ),
        "empty_object": {},
        "empty_array": [],
    }
    events = [
        {
            "seq": i,
            "type": ("login", "purchase", "refund", "view")[i % 4],
            "ok": i % 7 != 0,
            "amount": None if i % 4 != 1 else round(i * 3.25, 2),
            "user": {"id": 1000 + i, "name": "user-%d" % i, "tags": [WORDS[i % len(WORDS)], WORDS[(i * 3) % len(WORDS)]]},
            "message": " ".join(WORDS[(i + j) % len(WORDS)] for j in range(12 + i % 20)),
        }
        for i in range(250)
    ]
    large = {"report": "daily", "generated_by": "fixtures.py", "count": len(events), "events": events}
    return [
        ("json:sample:order", json.dumps(order, ensure_ascii=False)),
        ("json:sample:large", json.dumps(large)),
        # Starts like JSON, so it is classified as JSON, but does not parse:
        # the viewer must show the raw text, uncoloured, without an error.
        ("json:sample:truncated", '{"id": 42, "items": [1, 2, 3], "note": "cut off mid-'),
        ("json:sample:invalid", "{'id': 1, \"tags\": [\"a\", \"b\",], unquoted: true,}"),
    ]


def size_for(rng):
    """Log-ish size distribution: mostly small, a few fat keys."""
    roll = rng.random()
    if roll < 0.70:
        return rng.randint(1, 12)
    if roll < 0.95:
        return rng.randint(12, 120)
    return rng.randint(120, 1500)


def commands_for(rng, key, kind):
    n = size_for(rng)
    if kind == "string":
        if key.startswith("counter:"):
            return [("SET", key, rng.randint(0, 10_000_000))]
        if key.startswith("flag:"):
            return [("SET", key, rng.choice(["true", "false", "enabled", "off"]))]
        if key.startswith("lock:"):
            return [("SET", key, "held-by-worker-%d" % rng.randint(1, 32))]
        return [("SET", key, blob(rng, n * rng.choice([8, 32, 200])))]
    if kind == "json":
        obj = json_object(rng, key)
        # Roughly half pretty-printed, half compact — both are the same
        # string as far as Redis is concerned, and redis-pane's own JSON
        # viewer pretty-prints either on read, so this exercises both shapes
        # of what might actually be sitting in a real keyspace.
        body = json.dumps(obj, indent=2) if rng.random() < 0.5 else json.dumps(obj)
        return [("SET", key, body)]
    if kind == "hash":
        fields = []
        for i in range(min(n, 200)):
            fields += ["field:%s:%d" % (rng.choice(WORDS), i), sentence(rng, 3)]
        return [("HSET", key, *fields)]
    if kind == "list":
        return [("RPUSH", key, *[sentence(rng) for _ in range(min(n, 500))])]
    if kind == "set":
        return [("SADD", key, *{"%s-%d" % (rng.choice(WORDS), rng.randint(0, 9999)) for _ in range(min(n, 500))})]
    if kind == "zset":
        members = []
        for i in range(min(n, 500)):
            members += [round(rng.uniform(0, 10_000), 3), "player:%d" % rng.randint(1, 100_000)]
        return [("ZADD", key, *members)]
    if kind == "stream":
        out = []
        for _ in range(min(n, 100)):
            out.append(
                (
                    "XADD", key, "*",
                    "event", rng.choice(["login", "logout", "purchase", "refund", "view"]),
                    "user", rng.randint(1, 100_000),
                    "amount", round(rng.uniform(0, 500), 2),
                )
            )
        return out
    raise ValueError(kind)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    add_target_args(p)
    p.add_argument("-n", "--count", type=int, default=5000, help="keys to create (default 5000)")
    p.add_argument("--ttl-fraction", type=float, default=0.4,
                   help="share of keys given a %d-%ds TTL (default 0.4)" % (TTL_MIN_SECONDS, TTL_MAX_SECONDS))
    p.add_argument("--prefix", default="", help="prepend to every key name")
    p.add_argument("--flush", action="store_true", help="FLUSHDB first")
    p.add_argument("--seed", type=int, default=None, help="seed the RNG for a reproducible keyspace")
    p.add_argument("--batch", type=int, default=500, help="commands per pipeline flush")
    p.add_argument("--no-json-samples", action="store_true",
                   help="skip the fixed json:sample:* keys (long, large, truncated and invalid JSON)")
    args = p.parse_args()

    rng = random.Random(args.seed)
    r = connect(args)

    if args.flush:
        r.call("FLUSHDB")
        print("flushed db %d" % args.db)

    pending, made, expiring = [], 0, 0
    started = time.time()

    def flush():
        nonlocal pending
        for reply in r.pipeline(pending):
            if isinstance(reply, Exception):
                print("  ! %s" % reply, file=sys.stderr)
        pending = []

    for i in range(args.count):
        ns, kind = NAMESPACES[i % len(NAMESPACES)]
        key = "%s%s:%s" % (args.prefix, ns, blob(rng, 10) if rng.random() < 0.5 else rng.randint(1, 999_999))
        pending += commands_for(rng, key, kind)
        if rng.random() < args.ttl_fraction:
            pending.append(("EXPIRE", key, rng.randint(TTL_MIN_SECONDS, TTL_MAX_SECONDS)))
            expiring += 1
        made += 1
        if len(pending) >= args.batch:
            flush()
    if not args.no_json_samples:
        for name, body in json_samples():
            pending.append(("SET", args.prefix + name, body))
            made += 1
    flush()

    print(
        "created %d keys (%d with a %d-%ds TTL) in %.1fs; DBSIZE now %s"
        % (made, expiring, TTL_MIN_SECONDS, TTL_MAX_SECONDS, time.time() - started, r.call("DBSIZE"))
    )
    r.close()


if __name__ == "__main__":
    main()
