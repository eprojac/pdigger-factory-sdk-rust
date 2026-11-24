#!/bin/sh
# Writes empty sources for every target of the workspace manifests, so that Dockerfile.test can
# fetch and compile the dependencies in a layer of their own.
set -eu
mkdir -p crates/pdigger-factory-sdk/src crates/pdigger-factory-sdk/examples \
         crates/pdigger-factory-viz/src/bin crates/pdigger-factory-viz/examples
echo '//! stub' > crates/pdigger-factory-sdk/src/lib.rs
for example in testing_quickstart fake_server; do
  echo 'fn main() {}' > "crates/pdigger-factory-sdk/examples/$example.rs"
done
echo '//! stub' > crates/pdigger-factory-viz/src/lib.rs
echo 'fn main() {}' > crates/pdigger-factory-viz/src/bin/pdigger-factory-viewer.rs
# One example per crate at least, so that the examples' dependencies are compiled here too.
echo 'fn main() {}' > crates/pdigger-factory-viz/examples/screenshots.rs
