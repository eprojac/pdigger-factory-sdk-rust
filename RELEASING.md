# Releasing

A version tag publishes `pdigger-factory-sdk` and `pdigger-factory-viz` to crates.io. The
[release workflow](.github/workflows/release.yml) also uploads viewer binaries for Linux, macOS (arm64) and Windows.
Set the repository secret `CARGO_REGISTRY_TOKEN` to a token that can publish both crates.

## Prepare the release

1. Date the version heading in `CHANGELOG.md` and copy the file into both crates with
   `for c in crates/*/; do cp CHANGELOG.md "$c"; done`.
2. Update `workspace.package.version` and the version of `pdigger-factory-sdk` in `workspace.dependencies` in
   `Cargo.toml`.
3. Run `cargo update -w` and include `Cargo.lock` in the release commit. Builds use `--locked`.
4. Run `make test` and the [recording integration suite](TESTING.md). Commit the release files.
5. Tag that commit and push the tag. For version 0.1.0, use `git tag v0.1.0` and `git push origin v0.1.0`.

## What the workflow publishes

The workflow verifies the version, tests both crates and builds the viewer archives. It checks the release notes and
archives, runs `cargo publish --workspace --locked --dry-run` and compares any existing crate version with the
package SHA-256. A mismatch stops the job before upload.

After attaching the binaries to a draft GitHub release, it publishes the SDK first, then the viewer crate. Cargo
waits up to 60 seconds for the SDK to appear in the registry. The workflow retries up to five times at 30-second
intervals, then makes the GitHub release public.

If indexing, the second upload or the final GitHub step fails, choose **Re-run failed jobs** on the same run. The
workflow reuses the draft and skips matching crate versions already published.
