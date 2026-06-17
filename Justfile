set shell := ["bash", "-euo", "pipefail", "-c"]

# Codetracer-wasmi-recorder Justfile.
#
# This repo is a fork of upstream ``wasmi-labs/wasmi`` with
# CodeTracer recording hooks. The upstream project drives its CI
# through ``cargo test`` directly; this Justfile adds the
# ``build`` / ``test`` / ``lint`` aggregate recipes mandated by
# ``codetracer-specs/Repo-Requirements.md`` §1.3 + §2.4 so the
# shared CI dev-env workflow can re-play them under every flavor.

default:
  @just --list

# Build the wasmi_cli binary (the recorder's primary artefact).
build:
  cargo build --release --manifest-path crates/cli/Cargo.toml

build-debug:
  cargo build --manifest-path crates/cli/Cargo.toml

# Run the workspace test suite.
test:
  cargo test --workspace

t: test

lint:
  cargo fmt --check
  cargo clippy --workspace --all-targets -- -D warnings

format:
  cargo fmt --all

fmt: format
