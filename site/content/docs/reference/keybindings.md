---
title: Keybindings
---

> Generated from the keymap; do not edit. Regenerate with `UPDATE_DOCS=1 cargo test -p redis-pane-core --test docs_reference`.

These are the bindings. Keys can't be rebound from the config file. The help overlay (`?` or `F1`) and the hint bar show what the keys do in the context you are in. Where a key does a different thing depending on state, the row shows both, separated by `/`.

## Everywhere

Keys that work in every view while no prompt or dialog has the keyboard. While a prompt, the editor or a confirmation is open, only `F1` opens help.

| Key | Action |
| --- | --- |
| `Tab` | focus |
| `⌃R` | read-only |
| `g s` | slowlog |
| `g m` | monitor |
| `g p` | pub/sub |
| `g d` | dashboard |
| `Esc` | back |
| `q ⌃C` | quit |
| `? F1` | help |
| `g k` | keys |

## Key list

The left pane. With keys marked, `d` deletes all of them and `Esc` lets them go. Rows that mutate are refused under Read-only Mode.

| Key | Action |
| --- | --- |
| `→` | open / expand |
| `/` | filter / change filter |
| `s` | sort |
| `t` | tree view / flat view |
| `d` | delete |
| `c` | copy |
| `r` | rescan |
| `C` | copy redis-cli command |
| `R` | rename |
| `D` | duplicate key (COPY) |
| `Space` | mark |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom |
| `⏎` | open & move in |
| `← h` | collapse / parent |

### Filter prompt

Opened by `/`. Every other key is text.

| Key | Action |
| --- | --- |
| `⏎` | apply |
| `Esc` | clear & exit |

### Rename and duplicate prompt

Opened by `R` or `D` on a key.

| Key | Action |
| --- | --- |
| `⏎` | stage rename / stage duplicate |
| `Esc` | cancel |

## Value pane

The right pane, showing the open key. Each section lists the keys with the value cursor on (`Enter` puts it on a row). With the cursor off, plain movement drives the key list instead. Rows that mutate are refused under Read-only Mode.

### String

A String key, value cursor on.

| Key | Action |
| --- | --- |
| `e` | edit |
| `c` | copy |
| `t` | edit ttl |
| `r` | refetch |
| `C` | copy redis-cli command |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom |

### Hash

A Hash key, value cursor on.

| Key | Action |
| --- | --- |
| `e` | edit field |
| `a` | add field |
| `d` | delete field |
| `c` | copy |
| `t` | edit ttl |
| `r` | refetch |
| `C` | copy redis-cli command |
| `R` | rename field |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom |

### List

A List key, value cursor on.

| Key | Action |
| --- | --- |
| `e` | edit element |
| `a` | add element |
| `d` | delete element |
| `c` | copy |
| `t` | edit ttl |
| `r` | refetch |
| `C` | copy redis-cli command |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom |

### Set

A Set key, value cursor on.

| Key | Action |
| --- | --- |
| `a` | add member |
| `d` | remove member |
| `c` | copy |
| `t` | edit ttl |
| `r` | refetch |
| `C` | copy redis-cli command |
| `R` | rename member |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom |

### Sorted set

A Sorted set key, value cursor on.

| Key | Action |
| --- | --- |
| `e` | score |
| `a` | add member |
| `d` | remove member |
| `c` | copy |
| `t` | edit ttl |
| `r` | refetch |
| `C` | copy redis-cli command |
| `R` | rename member |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom |

### Stream

A Stream key, value cursor on.

| Key | Action |
| --- | --- |
| `c` | copy |
| `t` | edit ttl |
| `r` | refetch |
| `C` | copy redis-cli command |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom |

### JSON

A JSON key, value cursor on.

| Key | Action |
| --- | --- |
| `e` | edit |
| `c` | copy |
| `t` | edit ttl |
| `r` | refetch |
| `C` | copy redis-cli command |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom |

### Binary

A Binary key, value cursor on.

| Key | Action |
| --- | --- |
| `c` | copy |
| `t` | edit ttl |
| `r` | refetch |
| `C` | copy redis-cli command |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom |

## Editor

The inline editor, opened by `e`, `a` or `t` in the value pane.

### Value

Editing a whole String or JSON value.

| Key | Action |
| --- | --- |
| `⌃S` | stage |
| `⌃Z` | undo |
| `Esc` | cancel |
| `⌃Y` | redo |

### Field or element

Editing a Hash field, a List element, or capturing a Set member.

| Key | Action |
| --- | --- |
| `⏎` | stage |
| `⌃Z` | undo |
| `Esc` | cancel |
| `⌃Y` | redo |

### New Hash field: name

The first half of the add-field form.

| Key | Action |
| --- | --- |
| `⏎` | next: value |
| `Esc` | cancel |

### New Hash field: value

The second half of the add-field form.

| Key | Action |
| --- | --- |
| `⏎` | stage |
| `↑` | back to field |
| `⌃Z` | undo |
| `Esc` | cancel |
| `⌃Y` | redo |

### New sorted-set member: name

The first half of the add-member form.

| Key | Action |
| --- | --- |
| `⏎` | next: score |
| `Esc` | cancel |

### New sorted-set member: score

The second half of the add-member form.

| Key | Action |
| --- | --- |
| `⏎` | stage |
| `↑` | back to member |
| `⌃Z` | undo |
| `Esc` | cancel |
| `⌃Y` | redo |

### New List element

Adding an element at the head or tail.

| Key | Action |
| --- | --- |
| `⏎` | stage |
| `Tab` | head/tail |
| `⌃Z` | undo |
| `Esc` | cancel |
| `⌃Y` | redo |

### Sorted-set score

Editing an existing member's score.

| Key | Action |
| --- | --- |
| `⌃S` | stage |
| `⌃Z` | undo |
| `Esc` | cancel |
| `⌃Y` | redo |

### TTL

Editing the open key's TTL.

| Key | Action |
| --- | --- |
| `⌃S` | apply |
| `Esc` | cancel |

## Confirm

A staged mutation waits for you here. Under Read-only Mode it is refused at `y`.

| Key | Action |
| --- | --- |
| `y` | confirm |
| `Esc` | discard |

## Chords

A `g` followed by a second key opens a view. `Esc` cancels a pending chord.

| Key | Action |
| --- | --- |
| `g k` | keys |
| `g s` | slowlog |
| `g m` | monitor |
| `g p` | pub/sub |
| `g d` | dashboard |

## Dashboard

The server Dashboard, opened by a chord.

### Tiles

The tile grid, on a standalone server or after opening a node on a Cluster.

| Key | Action |
| --- | --- |
| `←→↑↓ hjkl` | move focus |
| `⏎` | expand tile |
| `r` | refetch |
| `c` | copy section |

### Node table (Cluster)

On a Cluster the Dashboard opens on a table of nodes. Opening one shows its tiles.

| Key | Action |
| --- | --- |
| `↑↓ jk` | move node |
| `⏎` | open node |
| `r` | refetch |

### Raw INFO

The overlay opened by expanding a tile.

| Key | Action |
| --- | --- |
| `↑↓ jk` | scroll |
| `c` | copy section |
| `r` | refetch |
| `Esc` | close |

## Monitor

The `MONITOR` tail. Pause shows only while the feed is open; reopen only once it has closed.

| Key | Action |
| --- | --- |
| `p` | pause |
| `/` | filter |
| `c` | copy command |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom (End resumes follow) |
| `r` | reopen |

## Pub/Sub

The subscription strip above the message tail. `Tab` switches between them.

### Subscription strip

The row of subscriptions. Unsubscribe and chip selection show once there is one.

| Key | Action |
| --- | --- |
| `Tab` | focus strip/tail |
| `a` | add |
| `d` | unsubscribe |
| `←→` | select chip |

### Message tail

The messages received. Pause shows only while the feed is open; reopen only once it has closed.

| Key | Action |
| --- | --- |
| `Tab` | focus strip/tail |
| `a` | add |
| `p` | pause |
| `/` | filter |
| `c` | copy payload |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom (End resumes follow) |
| `r` | reopen |

### Add subscription

Opened by `a`. Every other key is text.

| Key | Action |
| --- | --- |
| `⏎` | subscribe |
| `Tab` | sharded |
| `Esc` | cancel |

## Slowlog

The `SLOWLOG` view. Reset is refused under Read-only Mode.

| Key | Action |
| --- | --- |
| `s` | sort |
| `r` | refetch |
| `c` | copy command |
| `d` | reset slowlog |
| `↑↓ jk` | move |
| `PgUp/PgDn` | page |
| `Home/End` | top/bottom |

## Every binding

Every default single-key binding, one row per action, straight from the keymap. The same key does different things in different views; the action names the general meaning. The chords are in the table above.

| Key | Action |
| --- | --- |
| `Tab` | focus |
| `q ⌃C` | quit |
| `r` | refetch / rescan |
| `⌃R` | read-only |
| `? F1` | help |
| `Esc` | back |
| `↓ j` | move |
| `↑ k` | move |
| `PgDn` | page |
| `PgUp` | page |
| `Home` | top |
| `End` | bottom |
| `/` | filter |
| `s` | sort |
| `t` | tree / edit ttl |
| `→ l` | open / expand |
| `← h` | collapse / parent |
| `⏎` | open / move in value |
| `c` | copy |
| `C` | copy redis-cli command |
| `⌃→` | widen keys |
| `⌃←` | narrow keys |
| `d` | delete |
| `R` | rename |
| `D` | duplicate key (COPY) |
| `Space` | mark |
| `y` | confirm |
| `e` | edit |
| `a` | add |
| `⌃S` | stage |
| `⌃Z` | undo |
| `⌃Y` | redo |
| `p` | pause / resume |
