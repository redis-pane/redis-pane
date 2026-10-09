//! Startup refusals, run through the real binary. Docker-free: every case
//! exits before a connection is attempted.

#![cfg(unix)]

use std::process::{Command, Output};

fn run(config_home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_redis-pane"))
        .args(args)
        .env("XDG_CONFIG_HOME", config_home)
        .env_remove("REDIS_URL")
        .env_remove("REDIS_HOST")
        .env_remove("REDIS_PORT")
        .output()
        .expect("run redis-pane")
}

fn config_dir(contents: Option<&str>) -> tempdir::Dir {
    let dir = tempdir::Dir::new();
    if let Some(contents) = contents {
        let sub = dir.path().join("redis-pane");
        std::fs::create_dir_all(&sub).unwrap();
        let file = sub.join("config.json");
        std::fs::write(&file, contents).unwrap();
        // The config is refused when group- or world-readable (ADR-0002).
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    dir
}

mod tempdir {
    use std::path::{Path, PathBuf};
    pub struct Dir(PathBuf);
    impl Dir {
        pub fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "redis-pane-startup-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Dir(path)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

const CONFIG: &str = r#"{"profiles":{"staging":{"host":"cache-01","env":"staging"},"prod":{"host":"redis.prod","env":"prod"}}}"#;

#[test]
fn a_misspelled_profile_flag_exits_3_naming_it_and_the_available_ones() {
    let dir = config_dir(Some(CONFIG));
    let out = run(dir.path(), &["--profile", "stagin", "--print-target"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty(), "nothing is resolved");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("\"stagin\""), "{err}");
    assert!(err.contains("prod, staging"), "{err}");
}

#[test]
fn a_misspelled_positional_profile_exits_3_too() {
    let dir = config_dir(Some(CONFIG));
    let out = run(dir.path(), &["stagin", "--print-target"]);
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn a_profile_with_no_config_file_says_there_is_none() {
    let dir = config_dir(None);
    let out = run(dir.path(), &["--profile", "prod", "--print-target"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no config file"));
}

#[test]
fn a_correct_profile_still_resolves() {
    let dir = config_dir(Some(CONFIG));
    let out = run(dir.path(), &["--profile", "staging", "--print-target"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("from profile staging"));
}
