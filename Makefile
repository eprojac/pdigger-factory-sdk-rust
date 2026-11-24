# make test: the unit tests, the 100 % coverage gate, the golden tests and the examples, in
# Docker with no network (what CI runs). The other targets need a local Rust toolchain.
IMAGE := pdigger-factory-sdk-rust-test
# `make test DOCKER_BUILD_FLAGS=--no-cache` rebuilds the image from nothing.
DOCKER_BUILD_FLAGS ?=

.PHONY: test image integration integration-vars lint docs examples pictures viewer

test: image
	docker run --rm --network none $(IMAGE)

# make integration: the opt-in suites of TESTING.md in the test image. Set one or both:
#   PDIGGER_FACTORY_URL=http://<unit>:8120   checks a unit, read-only (observe=1 streams, dry_run=1 controls)
#   PDIGGER_FACTORY_RECORDINGS=<folder>      replays every <video>.world.ndjson.gz in it against its summary
integration: integration-vars image
	docker run --rm $(if $(PDIGGER_FACTORY_URL),-e PDIGGER_FACTORY_URL="$(PDIGGER_FACTORY_URL)",--network none) \
		$(if $(PDIGGER_FACTORY_RECORDINGS),-e PDIGGER_FACTORY_RECORDINGS=/recordings -v "$$(cd "$(PDIGGER_FACTORY_RECORDINGS)" && pwd)":/recordings:ro) \
		$(IMAGE) cargo test --locked -q -p pdigger-factory-sdk --test integration -- --ignored --nocapture

integration-vars:
	@test -n "$(PDIGGER_FACTORY_URL)$(PDIGGER_FACTORY_RECORDINGS)" || { echo 'make integration: set PDIGGER_FACTORY_URL=http://<unit>:8120 and/or PDIGGER_FACTORY_RECORDINGS=<folder>' >&2; exit 2; }
	@test -z "$(PDIGGER_FACTORY_RECORDINGS)" || test -d "$(PDIGGER_FACTORY_RECORDINGS)" || { echo 'make integration: $(PDIGGER_FACTORY_RECORDINGS) is not a folder' >&2; exit 2; }

image:
	docker build $(DOCKER_BUILD_FLAGS) -t $(IMAGE) -f Dockerfile.test .

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

docs:
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked

examples:
	sh scripts/run-examples.sh

# The pictures of the docs, from the fixture recordings dry1-0296-0306-full and dry1-0018-0153.
pictures:
	cargo run -p pdigger-factory-viz --example screenshots -- docs/img

# The viewer over a fixture recording (needs a display).
viewer:
	cargo run -p pdigger-factory-viz --release --bin pdigger-factory-viewer -- fixtures/recordings/dry1-0296-0306-full.ndjson.gz
