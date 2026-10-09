# Org task 2: community files

Part of [`org-followups-planning.md`](org-followups-planning.md).

## Context

The project now has its own org, docs site and Discussions. Reports should arrive with the
details `TESTING.md` asks for, questions should go to Discussions rather than issues, and the org
page should say what the project is.

## Deliverables

1. **Issue forms** in `.github/ISSUE_TEMPLATE/`, using GitHub's YAML issue forms:
   - **`bug.yml`, "Bug report".** Required fields:
     - the redis-pane version (`redis-pane --version`);
     - the OS and terminal (and tmux/SSH, if used);
     - the Redis server: version, and where it runs (local, Docker, Upstash, Redis Cloud, ElastiCache…);
     - the topology: standalone / Sentinel / Cluster;
     - the Environment shown in the title bar;
     - steps;
     - expected vs. actual.

     Optional: a screenshot, and the output of `redis-pane --probe` with secrets removed. Add a
     checkbox, "I removed passwords and hostnames I don't want public". Use the label `bug`.
   - **`feature.yml`, "Feature request":**
     - the problem, as "what were you trying to do";
     - the proposal;
     - "would an on-call engineer use this?", the PRD's test for every feature;
     - a note linking the book's Limits and non-goals page, so known non-goals aren't re-requested.

     Use the label `enhancement`.
   - **`config.yml`:**
     - `blank_issues_enabled: false`;
     - `contact_links` to Discussions (questions and ideas) at
       `https://github.com/redis-pane/redis-pane/discussions`;
     - `contact_links` to the docs at `https://redis-pane.github.io/redis-pane/`.
   - Create the `bug` and `enhancement` labels with `gh label create` if they're missing.
2. **The org profile README.** Create the public repo `redis-pane/.github` with
   `gh repo create redis-pane/.github --public`. Add `profile/README.md`:
   - one short paragraph on what redis-pane is;
   - links to the repo, the docs, the latest release and Discussions;
   - the demo GIF, referenced by its raw URL in `redis-pane/redis-pane`.

   Keep it under 25 lines. Commit it directly to that repo's `main`: it's a new, empty repo with
   no CI. Include the co-author line.
3. **`TESTING.md`'s "Reporting"** section points to the bug form
   (`…/issues/new?template=bug.yml`) and to Discussions.
4. **`docs/RELEASING.md`** gets a one-off checklist under "First release from the org", to remove
   once done:
   - `dist plan` resolves `redis-pane/redis-pane`;
   - the new release's installer URLs and asset links point at the org;
   - the README release badge shows the new version;
   - the old `vinodsantharam/redis-pane` installer URL still redirects.

## Testing

- `mdbook build book` stays clean, and `cargo test --workspace` still passes. Nothing in Rust
  should change.
- On the PR, open GitHub's "New issue" chooser preview if possible. If not, validate the YAML
  against the issue-forms schema: every field has an `id`, labels are lists, and so on.
- `https://github.com/redis-pane` shows the profile README after it is pushed. Check with
  `gh api repos/redis-pane/.github/contents/profile/README.md`.

## Out of scope

`CONTRIBUTING.md` (the book's Contributing page covers it), a code of conduct, funding, and any
settings change. Settings are done by the main session.
