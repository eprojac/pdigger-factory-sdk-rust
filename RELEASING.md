# Releasing

A release publishes both crates to crates.io and the viewer's binaries to a GitHub release. The workflow
`.github/workflows/release.yml` does it when a tag `v<version>` is pushed; it needs the repository secret
`CARGO_REGISTRY_TOKEN` (a crates.io token that may publish `pdigger-factory-sdk` and `pdigger-factory-viz`).

1. In `CHANGELOG.md`, give the version's section its date: `## <version> - Unreleased` becomes
   `## <version> - <date>` (the release notes are that section). Copy the file into both crates:
   `for c in crates/*/; do cp CHANGELOG.md "$c"; done`.
2. Set the version twice in the workspace `Cargo.toml`: `[workspace.package] version` and the
   `pdigger-factory-sdk = { path = ..., version = "<version>" }` line of `[workspace.dependencies]`.
3. Refresh the lock file, `cargo update -w`, and commit `Cargo.lock` with the rest: every build uses `--locked`.
4. Run `make test` (and `make integration` against the published recordings) and commit.
5. Tag and push: `git tag v<version> && git push origin v<version>`.

The workflow checks that the tag matches the version and runs the tests again, then builds the viewer for Linux,
macOS (arm64) and Windows. Only then does it publish, and every check that can fail comes before crates.io, which
cannot be undone: it checks the three viewer archives and the release notes, packages and builds both crates
(`cargo publish --workspace --dry-run`, the viewer against the SDK as it will be published), checks that a version on
crates.io already is this commit's package (the index's checksum against `cargo package`), and puts the binaries on a
draft GitHub release. Then it publishes `pdigger-factory-sdk` and `pdigger-factory-viz`: cargo waits up to 60 s for
crates.io to index the first, and the step tries up to 5 times, 30 s apart, publishing what is not on crates.io yet.
Last, the release goes public. What can still fail after the first upload (the second upload, crates.io's indexing,
making the release public) is finished by a re-run ("Re-run failed jobs"): a crate this tag's commit published already
is skipped, and the draft release is reused. A version on crates.io with other content (published by hand, or from a
tag pushed again at another commit) stops the run before anything is uploaded.
