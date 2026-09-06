# ============================================================
# MengDrive — build & release automation
#
#   make build         -> cargo build --release (binary for the image)
#   make docker-build  -> build image from the PREBUILT binary
#   make docker-push   -> tag + push image to Docker Hub
#   make release       -> build + docker-build + docker-push
#
# Image is built from the local release artifact only — no
# Rust toolchain inside the image, no source code shipped.
# ============================================================

IMAGE_NAME   ?= simenglabs/teledrive
IMAGE_TAG    ?= latest
FULL_IMAGE   := $(IMAGE_NAME):$(IMAGE_TAG)
BIN          := target/release/s3-telegram

.PHONY: help build check test docker-build docker-push release run clean

help:
	@echo "MengDrive make targets:"
	@echo "  make build         - cargo build --release (required before docker-build)"
	@echo "  make check         - cargo check (fast typecheck)"
	@echo "  make test          - run unit tests"
	@echo "  make docker-build  - build Docker image from the prebuilt binary"
	@echo "  make docker-push   - push image to Docker Hub ($(FULL_IMAGE))"
	@echo "  make release       - build + docker-build + docker-push"
	@echo "  make run           - run the release binary locally"

# 1) Local release build — this artifact is what goes into the image
build: $(BIN)

$(BIN): Cargo.toml Cargo.lock $(shell find src -name '*.rs' 2>/dev/null)
	cargo build --release

check:
	cargo check

test:
	cargo test

# 2) Docker image from the prebuilt binary (fails fast if not built)
docker-build: $(BIN)
	docker build -t $(FULL_IMAGE) .

# 3) Push to Docker Hub
docker-push: docker-build
	docker push $(FULL_IMAGE)

# Full pipeline
release: build docker-build docker-push
	@echo "Released $(FULL_IMAGE)"

run: $(BIN)
	./$(BIN)

clean:
	cargo clean
