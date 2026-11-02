# JSON readability (#144): syntax colours and soft-wrapped long lines

Status: **planned** 2026-11-02. One task, one Sonnet subagent, one PR (`Fixes #144`), merged when every check is green.


## Context

Issue #144 (the user, 2026-11-02): reading long JSON values in redis-pane at work is hard. Every
line is one plain colour, and long lines are cut off. Agreed with the user (2026-11-02):

- **Scope:** String values that are JSON, i.e. what `looks_like_json` already classifies. Not hash
  fields, list items, RedisJSON module keys or Pub/Sub payloads.
- **Option A only:** syntax colours plus readable long lines. No folding tree, no path breadcrumb.
- **The font item is dropped.** The terminal chooses the font. The issue gets a note saying so.
- **Long lines soft-wrap** under their indentation instead of being clipped with `…`.

This also delivers DESIGN §6.3's long-promised "String — syntax-highlighted when JSON … is
detected" for JSON. XML and YAML stay unbuilt, and DESIGN says so.

## Current code (verified)

- `crates/app/src/redis/read.rs:393` → `Value::Json(JsonValue::parse(&text))` when
  `looks_like_json`.
- `crates/core/src/state/value.rs:435`: `JsonValue { lines: Vec<String> }` pretty-printed by
  `serde_json::to_string_pretty`. Invalid JSON falls back to the raw lines. It implements `Viewer`
  (`measure`, `columns` = none, `row_count`, `row(i) -> Vec<String>`).
- `crates/core/src/render/mod.rs:~770–805` draws rows. Each cell goes through `clip(…, ellipsis)` in
  one token: `Text`, `Muted` for the first column of multi-column views, or `Selected` on the value
  cursor row.
- `StringValue::new(text, width)` + `value.rs` `wrap()`: the existing precedent for width-dependent
  rows. Follow the same pattern, including however a resize rebuilds it.
- Themes: `crates/core/src/theme/mod.rs` has `Token` (`COUNT = 22`), a palette per built-in theme
  (dark, light, high-contrast, 256 and monochrome slots), and contrast tests. Custom themes are
  parsed from config, and unknown fields are a parse error.

## Design

1. **Tokenizer, in the core, pure, no new dependency.** Split each pretty-printed line into spans:
   - `JsonKey`: a quoted string followed by `:`;
   - `JsonString`;
   - `JsonNumber`;
   - `JsonLiteral` (`true`, `false`, `null`);
   - `JsonPunct` (`{ } [ ] , :`);
   - leading whitespace, kept as is.

   Escaped quotes (`\"`) and unicode must not break a span. The invalid-JSON fallback lines are not
   tokenized: plain `Text`, as today.
2. **A styled-row seam on `Viewer`:** a default method `row_spans(i, now) -> Vec<Vec<(String, Token)>>`
   (cells → spans). It defaults to `row()` with each cell as one span in today's token, so no other
   viewer changes. `JsonValue` overrides it. The render loop draws spans instead of whole cells.
   The cursor row still paints everything in `Selected`, the existing single "row you're on"
   language.
3. **Soft-wrap.**
   - `JsonValue` keeps logical lines and is wrapped at the pane width into visual rows. Each
     continuation row is indented to the column where the line's value starts (after `"key": `, or
     after the leading whitespace for array items), with a floor so a narrow pane still progresses.
   - A string span split across rows keeps its token on every piece.
   - `row_count` counts visual rows, so scrolling and the cursor work unchanged on visual rows.
   - The header's `N lines` stays the logical line count.
   - Follow `StringValue`'s width-dependent construction and resize path.
4. **Theme tokens.** Add `json.key`, `json.string`, `json.number`, `json.literal` and `json.punct`
   to `Token`, with config names in that dotted form.
   - Set them in every built-in theme (truecolor and 256), chosen to pass the existing contrast
     tests.
   - **Monochrome:** keys bold, punctuation dim, the rest plain.
   - **Custom themes** may set them. When unset they fall back to `Text` (string, number, literal),
     `Muted` (punct) and `TypeJson` (key), so no existing config breaks or needs editing.
5. **Unchanged:** the editor (`ratatui-textarea`, plain text), liveness behaviour, copy (still
   copies the raw value), and the `json` type hue elsewhere.

## Files

- `crates/core/src/state/value.rs`: tokenizer, `JsonValue` wrap and spans, `Viewer::row_spans` default.
- `crates/core/src/render/mod.rs`: draw spans in the value body.
- `crates/core/src/theme/mod.rs` and the custom-theme config parsing (`crates/core/src/config/`):
  the new tokens, palettes and fallback.
- Docs, in the same PR:
  - DESIGN §6.3 and §5's token table;
  - `site/content/docs/viewing/types.mdx` (JSON);
  - `site/content/docs/appearance/custom-themes.md`, where every ```json block is parsed by the real
    config parser;
  - CHANGELOG `Unreleased`.

## Execution

The usual process:
- a plan doc, `docs/plans/json-highlighting.md`, written from this file;
- one Sonnet subagent and one PR, `Fixes #144`;
- merge only when every check is green;
- I'll comment on #144 that the font is the terminal's setting (after asking the user before
  posting);
- release as `1.0.1` only if the user says so.

## Verification

- **Tokenizer unit tests:** each kind; nested objects and arrays; escaped quotes; unicode;
  negative and exponent numbers; empty `{}` and `[]`; invalid JSON stays plain.
- **Wrap unit tests:** a long string wraps under its value column; a split span keeps its token;
  a narrow pane hits the floor; `row_count` equals the visual rows.
- **Golden frames:** a nested JSON value in dark, light, high-contrast and monochrome; a long
  string soft-wrapped at 80 columns; the value cursor on a highlighted row.
- **Theme contrast tests** cover the new tokens. A custom theme without them still parses, and
  renders with the fallbacks.
- **CI and checks:** `cargo test --workspace`, clippy, fmt, the boundary check, the perf suite (a
  large JSON value renders only the visible rows) and the docs `docs_reference` tests.
- **Manual:** `./scripts/redis-up.sh`, then `SET` a long nested JSON (with a long string field),
  `cargo run -p redis-pane`, and view it at 120 and 80 columns.
