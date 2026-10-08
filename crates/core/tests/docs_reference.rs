//! The generated keybindings reference (`book/src/reference/keybindings.md`)
//! and the config-example check for `book/src/reference/configuration.md`.
//!
//! The keybindings page is built from the same model the help overlay and the
//! hint bar read (`help::here`, `help::everywhere`, `Keymap::chords`), so it
//! cannot drift from them. It runs in the default Docker-free `cargo test`.
//! Regenerate with:
//!
//! ```text
//! UPDATE_DOCS=1 cargo test -p redis-pane-core --test docs_reference
//! ```
//!
//! File I/O lives here, in a test, never in the core's library code.

use std::path::PathBuf;

use redis_pane_core::config;
use redis_pane_core::help::{self, EditorContext, HelpContext, HelpRow, ValueContext};
use redis_pane_core::keymap::key_label;
use redis_pane_core::state::{
    FeedStatus, NameKind, OpenKey, PendingMutation, PubSubFocus, State, Subscription, Topology,
    View,
};

fn book_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../book/src/reference")
        .join(rel)
}

fn check_or_update(rel: &str, actual: &str) {
    let path = book_path(rel);
    if std::env::var_os("UPDATE_DOCS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        expected == actual,
        "docs reference out of date — run with `UPDATE_DOCS=1`:\n  UPDATE_DOCS=1 cargo test -p redis-pane-core --test docs_reference\n(file: {})",
        path.display()
    );
}

// ── building the page ───────────────────────────────────────────────────────

/// Rows from several representative states, merged by key in first-seen order.
/// Two states that bind the same key to different wording (`t` is "tree view"
/// in one and "flat view" in the other) become one row, `tree view / flat
/// view`. Refusals are dropped: the page describes bindings, not one state's
/// refusals.
fn merge(sets: Vec<Vec<HelpRow>>) -> Vec<(String, String)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for row in sets.into_iter().flatten() {
        let label = row.label.to_string();
        match out.iter_mut().find(|(k, _)| *k == row.keys) {
            Some((_, labels)) => {
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
            None => out.push((row.keys, vec![label])),
        }
    }
    out.into_iter().map(|(k, l)| (k, l.join(" / "))).collect()
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|")
}

fn table(out: &mut String, rows: Vec<(String, String)>) {
    out.push_str("| Key | Action |\n| --- | --- |\n");
    for (keys, label) in rows {
        out.push_str(&format!("| `{}` | {} |\n", cell(&keys), cell(&label)));
    }
    out.push('\n');
}

fn section(out: &mut String, level: usize, title: &str, intro: &str, rows: Vec<(String, String)>) {
    out.push_str(&format!("{} {title}\n\n{intro}\n\n", "#".repeat(level)));
    table(out, rows);
}

fn here(state: &State, ctx: HelpContext) -> Vec<HelpRow> {
    help::here(state, ctx)
}

fn on_screen(view: View) -> State {
    State {
        screen: view,
        ..State::default()
    }
}

fn open_state() -> State {
    State {
        open: Some(OpenKey::new(
            Some(0),
            "k".into(),
            redis_pane_core::state::value::Value::Str(
                redis_pane_core::state::value::StringValue::new("v", 40),
            ),
            -1,
            10,
            0,
        )),
        ..State::default()
    }
}

fn render_keybindings() -> String {
    let base = State::default();
    let mut out = String::new();
    out.push_str(
        "# Keybindings\n\n\
> Generated from the keymap; do not edit. Regenerate with `UPDATE_DOCS=1 cargo test -p redis-pane-core --test docs_reference`.\n\n\
These are the default bindings. The help overlay (`?` or `F1`) and the hint bar show the \
bindings in force, including any you have overridden. Where a key does a different thing \
depending on state, the row shows both, separated by `/`.\n\n",
    );

    // Everywhere: the Normal-mode globals, from the browser and from a view
    // (a view adds the way back to the key list).
    let mut globals = vec![help::everywhere(&open_state())];
    for view in [View::Slowlog, View::Monitor, View::PubSub, View::Dashboard] {
        globals.push(help::everywhere(&on_screen(view)));
    }
    section(
        &mut out,
        2,
        "Everywhere",
        "Keys that work in every view while no prompt or dialog has the keyboard. \
While a prompt, the editor or a confirmation is open, only `F1` opens help.",
        merge(globals),
    );

    // Key list.
    let mut keys_sets = Vec::new();
    for (tree, filtered) in [(false, false), (true, false), (false, true)] {
        keys_sets.push(here(&base, HelpContext::Keys { tree, filtered }));
    }
    section(
        &mut out,
        2,
        "Key list",
        "The left pane. With keys marked, `d` deletes all of them and `Esc` lets them go. \
Rows that mutate are refused under Read-only Mode.",
        merge(keys_sets),
    );
    section(
        &mut out,
        3,
        "Filter prompt",
        "Opened by `/`. Every other key is text.",
        merge(vec![here(&base, HelpContext::Filter)]),
    );
    section(
        &mut out,
        3,
        "Rename and duplicate prompt",
        "Opened by `R` or `D` on a key.",
        merge(vec![
            here(&base, HelpContext::Rename(NameKind::Rename)),
            here(&base, HelpContext::Rename(NameKind::Copy)),
        ]),
    );

    // Value pane, one section per type, cursor on.
    out.push_str(
        "## Value pane\n\nThe right pane, showing the open key. Each section lists the keys \
with the value cursor on (`Enter` puts it on a row). With the cursor off, plain movement \
drives the key list instead. Rows that mutate are refused under Read-only Mode.\n\n",
    );
    let types: [(&str, ValueContext); 8] = [
        ("String", ValueContext::Str { cursor: true }),
        ("Hash", ValueContext::Hash { cursor: true }),
        ("List", ValueContext::List { cursor: true }),
        ("Set", ValueContext::Set { cursor: true }),
        ("Sorted set", ValueContext::ZSet { cursor: true }),
        ("Stream", ValueContext::Stream { cursor: true }),
        ("JSON", ValueContext::Json { cursor: true }),
        ("Binary", ValueContext::Binary { cursor: true }),
    ];
    for (name, ctx) in types {
        section(
            &mut out,
            3,
            name,
            &format!("A {name} key, value cursor on."),
            merge(vec![here(&base, HelpContext::Value(Some(ctx)))]),
        );
    }

    // Editor.
    out.push_str(
        "## Editor\n\nThe inline editor, opened by `e`, `a` or `t` in the value pane.\n\n",
    );
    let editors: [(&str, &str, EditorContext); 8] = [
        (
            "Value",
            "Editing a whole String or JSON value.",
            EditorContext::Value,
        ),
        (
            "Field or element",
            "Editing a Hash field, a List element, or capturing a Set member.",
            EditorContext::Field,
        ),
        (
            "New Hash field: name",
            "The first half of the add-field form.",
            EditorContext::AddFormName { zset: false },
        ),
        (
            "New Hash field: value",
            "The second half of the add-field form.",
            EditorContext::AddFormValue { zset: false },
        ),
        (
            "New sorted-set member: name",
            "The first half of the add-member form.",
            EditorContext::AddFormName { zset: true },
        ),
        (
            "New sorted-set member: score",
            "The second half of the add-member form.",
            EditorContext::AddFormValue { zset: true },
        ),
        (
            "New List element",
            "Adding an element at the head or tail.",
            EditorContext::ListAdd,
        ),
        (
            "Sorted-set score",
            "Editing an existing member's score.",
            EditorContext::ZSetScore,
        ),
    ];
    for (title, intro, ctx) in editors {
        section(
            &mut out,
            3,
            title,
            intro,
            merge(vec![here(&base, HelpContext::Editor(ctx))]),
        );
    }
    section(
        &mut out,
        3,
        "TTL",
        "Editing the open key's TTL.",
        merge(vec![here(&base, HelpContext::Editor(EditorContext::Ttl))]),
    );

    // Confirm.
    let staged = State {
        confirm: Some(PendingMutation::DeleteKey {
            index: 0,
            name: "k".into(),
        }),
        ..State::default()
    };
    section(
        &mut out,
        2,
        "Confirm",
        "A staged mutation waits for you here. Under Read-only Mode it is refused at `y`.",
        merge(vec![here(&staged, HelpContext::Confirm)]),
    );

    // Chords.
    let chord_rows = base
        .keymap
        .chords()
        .iter()
        .map(|c| {
            (
                format!("{} {}", key_label(&c.prefix), key_label(&c.second)),
                c.action.label().to_string(),
            )
        })
        .collect();
    section(
        &mut out,
        2,
        "Chords",
        "A `g` followed by a second key opens a view. `Esc` cancels a pending chord.",
        chord_rows,
    );

    // Dashboard.
    out.push_str("## Dashboard\n\nThe server Dashboard, opened by a chord.\n\n");
    let dash = on_screen(View::Dashboard);
    section(
        &mut out,
        3,
        "Tiles",
        "The tile grid, on a standalone server or after opening a node on a Cluster.",
        merge(vec![here(&dash, HelpContext::Dashboard)]),
    );
    let cluster = State {
        screen: View::Dashboard,
        connection: redis_pane_core::state::Connection {
            topology: Some(Topology {
                primaries: 3,
                nodes: 6,
            }),
            ..Default::default()
        },
        ..State::default()
    };
    section(
        &mut out,
        3,
        "Node table (Cluster)",
        "On a Cluster the Dashboard opens on a table of nodes. Opening one shows its tiles.",
        merge(vec![here(&cluster, HelpContext::DashboardCluster)]),
    );
    section(
        &mut out,
        3,
        "Raw INFO",
        "The overlay opened by expanding a tile.",
        merge(vec![here(&dash, HelpContext::DashboardOverlay)]),
    );

    // Monitor.
    let mut monitor_open = on_screen(View::Monitor);
    monitor_open.monitor.status = FeedStatus::Open;
    let mut monitor_closed = on_screen(View::Monitor);
    monitor_closed.monitor.status = FeedStatus::Closed { reason: None };
    section(
        &mut out,
        2,
        "Monitor",
        "The `MONITOR` tail. Pause shows only while the feed is open; reopen only once it has closed.",
        merge(vec![
            here(&monitor_open, HelpContext::Monitor),
            here(&monitor_closed, HelpContext::Monitor),
        ]),
    );

    // Pub/Sub.
    let mut strip = on_screen(View::PubSub);
    strip
        .pubsub
        .subscriptions
        .push(Subscription::Channel("c".into()));
    let mut tail_open = strip.clone();
    tail_open.pubsub.status = FeedStatus::Open;
    let mut tail_closed = strip.clone();
    tail_closed.pubsub.status = FeedStatus::Closed { reason: None };
    out.push_str("## Pub/Sub\n\nThe subscription strip above the message tail. `Tab` switches between them.\n\n");
    section(
        &mut out,
        3,
        "Subscription strip",
        "The row of subscriptions. Unsubscribe and chip selection show once there is one.",
        merge(vec![here(
            &strip,
            HelpContext::PubSub {
                focus: PubSubFocus::Strip,
            },
        )]),
    );
    section(
        &mut out,
        3,
        "Message tail",
        "The messages received. Pause shows only while the feed is open; reopen only once it has closed.",
        merge(vec![
            here(
                &tail_open,
                HelpContext::PubSub {
                    focus: PubSubFocus::Tail,
                },
            ),
            here(
                &tail_closed,
                HelpContext::PubSub {
                    focus: PubSubFocus::Tail,
                },
            ),
        ]),
    );
    section(
        &mut out,
        3,
        "Add subscription",
        "Opened by `a`. Every other key is text.",
        merge(vec![here(&strip, HelpContext::PubSubAdding)]),
    );

    // Slowlog.
    section(
        &mut out,
        2,
        "Slowlog",
        "The `SLOWLOG` view. Reset is refused under Read-only Mode.",
        merge(vec![here(&on_screen(View::Slowlog), HelpContext::Slowlog)]),
    );

    // Every binding, straight from the keymap: the contextual tables above
    // show only what help shows, so a binding help never mentions (`⏎`, `←`,
    // the pane-width keys, redo) still has a home here.
    let mut by_action: Vec<(redis_pane_core::keymap::Action, Vec<String>)> = Vec::new();
    for b in base.keymap.bindings() {
        match by_action.iter_mut().find(|(a, _)| *a == b.action) {
            Some((_, keys)) => keys.push(key_label(&b.key)),
            None => by_action.push((b.action, vec![key_label(&b.key)])),
        }
    }
    section(
        &mut out,
        2,
        "Every binding",
        "Every default single-key binding, one row per action, straight from the keymap. \
The same key does different things in different views; the action names the general \
meaning. The chords are in the table above.",
        by_action
            .into_iter()
            .map(|(a, keys)| (keys.join(" "), a.label().to_string()))
            .collect(),
    );

    // Trim the trailing blank line: the file ends in exactly one newline.
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[test]
fn keybindings_reference_is_current() {
    check_or_update("keybindings.md", &render_keybindings());
}

#[test]
fn keybindings_reference_is_byte_stable() {
    assert_eq!(render_keybindings(), render_keybindings());
}

// ── config examples ─────────────────────────────────────────────────────────

/// Every ```` ```json ```` block in the page, with whether it is expected to
/// fail (```` ```json,invalid ````).
fn json_blocks(text: &str) -> Vec<(bool, usize, String)> {
    let mut blocks = Vec::new();
    let mut current: Option<(bool, usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        match &mut current {
            None => {
                let fence = line.trim_end();
                if fence == "```json" {
                    current = Some((false, i + 1, String::new()));
                } else if fence == "```json,invalid" {
                    current = Some((true, i + 1, String::new()));
                }
            }
            Some((_, _, body)) => {
                if line.trim_end() == "```" {
                    blocks.push(current.take().unwrap());
                } else {
                    body.push_str(line);
                    body.push('\n');
                }
            }
        }
    }
    assert!(current.is_none(), "unterminated ```json block");
    blocks
}

/// Every `.md` file under `book/src`, so a ```` ```json ```` block in any chapter
/// is held to the real parser, not only the ones in the reference.
fn book_pages() -> Vec<PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("book/src must be readable") {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "md") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&book_path("..").canonicalize().unwrap(), &mut out);
    out.sort();
    out
}

#[test]
fn config_examples_parse_with_the_real_parser() {
    let reference = std::fs::read_to_string(book_path("configuration.md"))
        .expect("book/src/reference/configuration.md must exist");
    assert!(
        json_blocks(&reference)
            .iter()
            .any(|(invalid, _, _)| !invalid),
        "configuration.md must contain at least one valid ```json example"
    );
    for page in book_pages() {
        let text = std::fs::read_to_string(&page).unwrap();
        for (invalid, line, body) in json_blocks(&text) {
            let result = config::parse(&body);
            let at = format!("{}:{line}", page.display());
            if invalid {
                assert!(
                    result.is_err(),
                    "{at}: a `json,invalid` example parsed, but should be refused"
                );
            } else {
                assert!(
                    result.is_ok(),
                    "{at}: example does not parse: {:?}",
                    result.err()
                );
            }
        }
    }
}

#[test]
fn the_invalid_fence_is_recognised() {
    let blocks = json_blocks("```json,invalid\n{\"passwrod\": 1}\n```\n```json\n{}\n```\n");
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].0 && config::parse(&blocks[0].2).is_err());
    assert!(!blocks[1].0 && config::parse(&blocks[1].2).is_ok());
}
