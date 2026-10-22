# Releasing

The release steps. The tag push is the only irreversible one, and it waits for the user's go-ahead.

1. **Close out the milestone** in its close-out plan (the latest is
   [`plans/m6-close-out.md`](plans/m6-close-out.md)): PLAN, PRD and DESIGN catch up with what shipped.
2. **Bump the version** in the root `Cargo.toml` (`[workspace.package] version`) and let
   `Cargo.lock` follow.
3. **Re-pin the installer URLs** (`.../releases/download/vX/redis-pane-installer.{sh,ps1}`) in:
   - `README.md`
   - `TESTING.md`, including the version in its first paragraph
   - `site/content/docs/getting-started/installation.md`
   - `site/lib/release.ts`, the landing page's version and installer URLs (one constant)
   - the version named in the README's Compatibility section
4. **Update `CHANGELOG.md`.** Rename `Unreleased` to the new version and date, add a fresh empty
   `Unreleased` above it, and update the compare links at the bottom.
5. **Check:** `cargo test --workspace`, clippy, `cargo fmt --all -- --check`, and `pnpm --dir site build` (it builds the docs site).
   `grep -rn "<old version>"` finds anything missed.
6. **Merge** the release PR.
7. **Tag the merge commit and push the tag**, only after the user says so. The `Release` workflow
   (cargo-dist) builds the assets.
8. **Verify the assets:** `gh release view <tag> --json assets` lists the five platform archives (macOS arm64 and x86_64,
   Linux x86_64 and aarch64 musl, Windows x86_64), their checksums and both installers. Run the
   installer and confirm `redis-pane --version` prints the new version. Run it on a current host,
   and in an `amazonlinux:2` container.
   Also check the Homebrew tap:
   - `gh api repos/redis-pane/homebrew-tap/commits --jq '.[0].commit.message'` shows a formula
     commit for the new version (the `publish-homebrew-formula` job in the Release run);
   - `brew install redis-pane/tap/redis-pane` works on a clean machine, and
     `redis-pane --version` prints the new version.
9. **Re-measure the download sizes** in the asset-size footnote (README `[^2]`,
   `site/content/docs/index.mdx` and `site/app/(home)/page.tsx`) from
   `gh release view <tag> --json assets`, and name the new release in it.

## Renewing `HOMEBREW_TAP_TOKEN`

The Release workflow pushes the formula to `redis-pane/homebrew-tap` with the repo secret
`HOMEBREW_TAP_TOKEN`. It is a fine-grained token with an expiry date, scoped to the
`homebrew-tap` repo alone with Contents read and write. When it expires, the
`publish-homebrew-formula` job fails loudly. Create a new token with the same scope, then:

```bash
gh secret set HOMEBREW_TAP_TOKEN -R redis-pane/redis-pane
```

`publish-prereleases = true` in `dist-workspace.toml` makes the beta tags publish to the tap.
At 1.0 it can be turned off so only stable tags reach the tap; the tap is then verified on the
first beta tag, and `1.0.0` is stable either way.
