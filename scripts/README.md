# Dev scripts

A disposable Redis and something to put in it, so the TUI can be developed
against a keyspace that moves. Nothing here is part of the build or the test
suites (ADR-0011's integration suite uses `testcontainers` instead); these are
for driving the app by hand.

Python 3 only — no virtualenv, no pip. `resp.py` is a ~100-line RESP2 client
that the two tools share.

```bash
./scripts/redis-up.sh                        # start redis:8.4-alpine on :6379
python3 scripts/fixtures.py --flush          # 5000 keys, all types, 40% with TTLs
cargo run -p redis-pane                      # browse it
python3 scripts/churn.py                     # in a second terminal: keep it moving
./scripts/redis-up.sh down                   # throw it away
```

## `redis-up.sh`

`up` (default) / `down` / `restart` / `cli` / `logs` / `status`. Persistence is
off — the data is disposable, and an unexpected background save is a `-MISCONF`
in the middle of a demo. Override `REDIS_PANE_PORT`, `REDIS_PANE_CONTAINER` or
`REDIS_PANE_IMAGE` in the environment; the image pin can drop to `redis:6.2` to
test the server floor (ADR-0007).

## `fixtures.py`

Writes strings, hashes, lists, sets, sorted sets and streams across ~15
namespaces, with sizes spread over two orders of magnitude so the memory column
is not a flat line. `--ttl-fraction` (default 0.4) of the keys get a TTL
between **10s and 2m**, which is short enough that the countdown visibly moves
and keys really do disappear while you watch.

```bash
python3 scripts/fixtures.py -n 200000            # enough to watch SCAN stream
python3 scripts/fixtures.py --ttl-fraction 1     # everything expires
python3 scripts/fixtures.py --seed 7 --flush     # reproducible keyspace
```

## `churn.py`

Deletes, mutates, expires and creates keys at random, type-appropriately, at a
given rate. This is how liveness gets watched rather than reasoned about: open
a key in redis-pane and the Viewer should follow the server with no refresh
(ADR-0006), and the keys pane should gain and lose rows under the cursor.

```bash
python3 scripts/churn.py --rate 40 --duration 60 # a burst
python3 scripts/churn.py --focus user:1234       # hammer the key you have open
python3 scripts/churn.py --no-delete --no-create # mutations only
```

Both tools take `--host/--port/--username/--password/--db`.

The demo recordings are rendered from `tapes/` (see `tapes/README.md`); they use these two tools against their own
container on port 6390, never the one above.

## A local Redis Cluster

`fixtures.py` and `churn.py` speak to one node and do not follow `MOVED`, so they cannot fill a
Cluster; use `redis-cli -c` for that. This brings up 3 primaries and 3 replicas in one container,
on host ports 7100-7105 (7000 is taken by AirPlay Receiver on macOS; the nodes announce `127.0.0.1`, so the same ports must be published, which
is why they are not remapped):

```bash
docker run -d --name redis-pane-cluster $(for p in 7100 7101 7102 7103 7104 7105; do printf -- '-p %s:%s ' $p $p; done) \
  redis:8.4-alpine sh -c 'for p in 7100 7101 7102 7103 7104 7105; do mkdir -p /data/$p; \
    redis-server --port $p --cluster-enabled yes --cluster-config-file /data/$p/nodes.conf --dir /data/$p \
    --cluster-announce-ip 127.0.0.1 --cluster-node-timeout 3000 --bind 0.0.0.0 --protected-mode no \
    --appendonly no --save "" --daemonize yes; done; echo started; exec tail -f /dev/null'
docker exec redis-pane-cluster redis-cli --cluster create \
  127.0.0.1:7100 127.0.0.1:7101 127.0.0.1:7102 127.0.0.1:7103 127.0.0.1:7104 127.0.0.1:7105 \
  --cluster-replicas 1 --cluster-yes
for i in $(seq 1 300); do docker exec redis-pane-cluster redis-cli -c -p 7100 set key:$i v >/dev/null; done
cargo run -p redis-pane -- --url redis-cluster://127.0.0.1:7100
docker rm -f redis-pane-cluster                # throw it away
```

## `ssh-box.sh`: redis-pane over SSH

A bastion in a box, for checking the paths that only run inside an SSH session. One container
holds sshd, a redis-pane built from this checkout, and a Redis bound to the container's own
loopback. Only SSH is published, on `127.0.0.1:2222`, so the one way to reach the keyspace is to
ssh in and run redis-pane there, the way it runs on a real bastion host.

```bash
./scripts/ssh-box.sh up        # build from the checkout, start, seed 5000 keys
./scripts/ssh-box.sh ssh       # ssh in and run redis-pane (extra args go to it)
./scripts/ssh-box.sh shell     # ssh in to a shell: fixtures, churn, tmux, redis-cli
./scripts/ssh-box.sh restart   # rebuild after a code change
./scripts/ssh-box.sh down      # throw it away
```

What to check over SSH:

- **Copying.** `c` uses OSC 52, because sshd sets `SSH_TTY` (ADR-0013). Paste on your own
  machine. Run `tmux` in `shell` to try the common bastion case, where it most often goes missing
  (the box's tmux has `set-clipboard on`).
- **The locale fallback.** sshd accepts `LANG` and `LC_*`, as Debian and Ubuntu do, so your
  locale is forwarded. `env -u LANG -u LC_ALL -u LC_CTYPE ./scripts/ssh-box.sh ssh` forwards none,
  and the frame should draw in ASCII.
- **Liveness.** Open a key, then `churn --focus <key>` from a `shell` in a second terminal.

The login uses a throwaway key in `target/ssh-box/`. `FIXTURE_KEYS` sets how many keys are seeded,
and `REDIS_PANE_SSH_PORT` sets the host port. The binary is a musl build, not the shipped
`x86_64-unknown-linux-gnu` one, which keeps the box on the same `redis:8.4-alpine` as `redis-up.sh`.
