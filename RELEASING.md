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
   Point the install lines at the new tag: `--tag` and `tag =` in `README.md`, both crate READMEs, `docs/viz.md`,
   `docs/testing.md` and `docs/tutorials/04-rust.md`.
4. Run `make test` and the [recording integration suite](TESTING.md). Commit the release files.
5. Tag that commit and push the tag. For version 1.0.0, use `git tag v1.0.0` and `git push origin v1.0.0`.

## What the workflow publishes

The workflow verifies the version, tests both crates and builds the viewer archives. It checks the release notes and
archives, runs `cargo publish --workspace --locked --dry-run` and compares any existing crate version with the
package SHA-256. A mismatch stops the job before upload.

After attaching the binaries to a draft GitHub release, it publishes the SDK first, then the viewer crate. Cargo
waits up to 60 seconds for the SDK to appear in the registry. The workflow retries up to five times at 30-second
intervals, then makes the GitHub release public.

If indexing, the second upload or the final GitHub step fails, choose **Re-run failed jobs** on the same run. The
workflow reuses the draft and skips matching crate versions already published.

## The viewer binaries

The macOS and Windows viewers are built on the runners. The Linux viewer is built in a manylinux_2_28 container
([viewer-linux.yml](.github/workflows/viewer-linux.yml), AlmaLinux 8), so it runs on glibc 2.28 and later (RHEL 8,
Debian 10, Ubuntu 20.04 and newer). The job fails if the binary needs a newer glibc. None of the binaries is signed or
notarised.

To rebuild an existing release's Linux archive, run the workflow **Rebuild a release's Linux viewer**
([viewer-linux-rebuild.yml](.github/workflows/viewer-linux-rebuild.yml)) with the tag. It builds the archive the same
way and uploads it over the one on that release. With **replace** off it only builds: the archive stays the run's
artifact `viewer-x86_64-unknown-linux-gnu`. To check that archive before it reaches the release, download it with
`gh run download <run id> -n viewer-x86_64-unknown-linux-gnu` and upload that file with
`gh release upload <tag> pdigger-factory-viewer-<tag>-x86_64-unknown-linux-gnu.tar.gz --clobber`. A second run would
build it again. The Linux archives first released with 1.0.0 and 1.0.1 were built on the runner and needed glibc 2.43.

## Publish on GitHub only

Before pushing the tag, set the repository Actions variable `SDK_GITHUB_ONLY_TAG` to the full tag name, including
the leading `v`. The workflow tests the tag and builds the viewer for all three platforms. Download the three
`viewer-*` artifacts and attach the archives inside them when you publish the GitHub release.
