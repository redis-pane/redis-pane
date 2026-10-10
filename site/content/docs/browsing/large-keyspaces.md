---
title: "Large keyspaces"
---

`redis-pane` is built to stay responsive with a million keys. It holds the key names compactly
(in a test, a million keys of ordinary length take about 40 MB) and only draws the rows on
screen.

## The key limit

The Loaded set holds up to **2,000,000 keys**. When the scan reaches that, it stops. A
persistent warning appears above the list:

```text
⚠ 2,000,000 key limit reached — narrow the filter
```

This matters because everything you see is then a **prefix** of the keyspace, not all of it. A
missing key may just be one that the scan didn't reach. The warning stays through filtering,
sorting and switching between tree and flat, none of which rescan.

A filter only searches the keys already loaded, so it can't reach past the limit. To look for a
key beyond it, use `redis-cli --scan --pattern 'prefix:*'` on the same server.

## `rebuilding N%`

Changing the filter, the sort or the tree on a very large list is real work. It's done in small
slices between frames, so the screen never freezes:

- The **old list stays on screen** and fully usable while the new one is built.
- The status bar says `rebuilding 42%` in place of the sort readout.
- When the new list is ready, it swaps in at once, and your cursor keeps its place.

You see this readout only past about 32,000 keys. Below that, the change is instant. In the
project's tests, a tree toggle or a sort change at a million keys finishes in about a tenth of a
second.

Typing in the filter box always shows your text at once. Only the rows lag.

## What `Esc` does

`Esc` stops a running **scan**. It doesn't stop a rebuild. If you've started one you don't want,
change the filter or sort again, which replaces it. See [Limits](../limits.md).
