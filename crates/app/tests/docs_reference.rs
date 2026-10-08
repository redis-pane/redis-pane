//! The generated CLI reference (`book/src/reference/cli.md`) is checked here,
//! in the default Docker-free run, so it cannot drift from the clap
//! definition. Regenerate with:
//!
//! ```text
//! UPDATE_DOCS=1 cargo test -p redis-pane --test docs_reference
//! ```

use std::path::PathBuf;

fn page_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../book/src/reference/cli.md")
}

fn render() -> String {
    let help = redis_pane::cli::cli_command()
        .term_width(100)
        .render_long_help()
        .to_string();
    format!(
        "# Command-line options\n\
\n\
> Generated from the `clap` definition; do not edit. Regenerate with `UPDATE_DOCS=1 cargo test -p redis-pane --test docs_reference`.\n\
\n\
`redis-pane` takes a target from flags, or finds one. With no arguments it uses, in order:\n\
\n\
1. flags (`--profile`, `--url`, `--host`, and the rest, below)\n\
2. the default Profile in the config file\n\
3. the environment (`REDIS_URL`, or the discrete `REDIS_HOST`/`REDIS_PORT`/`REDIS_USER`/`REDIS_PASSWORD`)\n\
4. `127.0.0.1:6379`\n\
\n\
It never prompts. The title bar always shows the target and where it came from.\n\
\n\
```text\n\
{}\n\
```\n",
        help.trim_end()
    )
}

#[test]
fn cli_reference_is_current() {
    let actual = render();
    let path = page_path();
    if std::env::var_os("UPDATE_DOCS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        expected == actual,
        "docs reference out of date — run with `UPDATE_DOCS=1`:\n  UPDATE_DOCS=1 cargo test -p redis-pane --test docs_reference\n(file: {})",
        path.display()
    );
}
