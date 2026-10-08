# Releasing

The release steps. The tag push is the only irreversible one, and it waits for the user's go-ahead.

1. **Close out the milestone** in its close-out plan (the latest is
   [`plans/m6-close-out.md`](plans/m6-close-out.md)): PLAN, PRD and DESIGN catch up with what shipped.
2. **Bump the version** in the root `Cargo.toml` (`[workspace.package] version`) and let
   `Cargo.lock` follow.
3. **Re-pin the installer URLs** (`.../releases/download/vX/redis-pane-installer.{sh,ps1}`) in:
   - `README.md`
   - `TESTING.md`, including the version in its first paragraph
   - `book/src/getting-started/installation.md`
   - the version named in the README's Compatibility section
4. **Update `CHANGELOG.md`.** Rename `Unreleased` to the new version and date, add a fresh empty
   `Unreleased` above it, and update the compare links at the bottom.
5. **Check:** `cargo test --workspace`, clippy, `cargo fmt --all -- --check`, and `mdbook build book`.
   `grep -rn "<old version>"` finds anything missed.
6. **Merge** the release PR.
7. **Tag the merge commit and push the tag**, only after the user says so. The `Release` workflow
   (cargo-dist) builds the assets.
8. **Verify the assets:** `gh release view <tag> --json assets` lists the four platform archives,
   their checksums and both installers. Run the installer and confirm `redis-pane --version`
   prints the new version.
