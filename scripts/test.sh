#!/bin/sh
# The unit-test run inside the Docker image (no network): every test, the golden tests, the
# 100 % line and function coverage gate, the doc tests, then the examples (which the READMEs and
# docs quote) over the fixtures and against the fake unit on 127.0.0.1.
set -eu
cd /src
export CARGO_NET_OFFLINE=true
# The crates carry the repository's LICENSE, NOTICE and CHANGELOG.md (CI's lint job checks the same).
for crate in crates/*/; do
  for file in LICENSE NOTICE CHANGELOG.md; do cmp "$file" "$crate$file"; done
done
# The third-party licences the release workflow puts beside the viewer binary (NOTICE names the file).
python3 scripts/third-party-licenses.py --offline > /tmp/THIRD-PARTY-LICENSES.txt
grep -q 'epaint_default_fonts .* (fonts/OFL.txt)' /tmp/THIRD-PARTY-LICENSES.txt
# Window glue only (eframe::run_native needs a display): listed in TESTING.md.
IGNORE='crates/pdigger-factory-viz/src/(bin/|native\.rs)'
cargo llvm-cov --workspace --all-targets --locked --offline \
  --ignore-filename-regex "$IGNORE" \
  --show-missing-lines \
  --fail-under-lines 100 --fail-under-functions 100
cargo test --doc --workspace --all-features --locked --offline
sh scripts/run-examples.sh
echo "make test: all tests passed, coverage 100 % (lines, functions), doc tests and examples ran"
