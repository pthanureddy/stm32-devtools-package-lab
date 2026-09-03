# STM32 DevTools Package Lab

A Rust and TypeScript developer-tools lab for package metadata, deterministic
distribution assembly, and machine-readable integration. The Rust CLI validates
STM32-style workspace manifests, resolves dependency order, and repacks an
existing host-side distribution with product-specific files and configuration.

This independent learning project is not affiliated with STMicroelectronics.
It models developer-tooling and release-engineering concerns; it does not build,
flash, or validate target firmware or hardware.

## Capabilities

- Parse and validate YAML or JSON package manifests, semantic versions, MCU
  families, dependency references, artifact checksums, and toolchain metadata.
- Produce a deterministic topological installation plan as JSON.
- Merge a product overlay over a base distribution with explicit precedence.
- Emit a reproducible `tar.gz`, a sorted per-file SHA-256/size/mode/provenance
  manifest, and a byte-identical sidecar manifest.
- Reject unsafe paths, links, special files, ambiguous file/directory
  collisions, overlapping inputs, and outputs placed inside source trees.
- Stage payload bytes while hashing and publish via temporary files.
- Read, validate, and summarize the workspace manifest format from TypeScript.
- Exercise Rust, TypeScript, Python verification, reproducibility, and dependency
  audit gates in GitHub Actions.

## Quick start

The repository pins Rust in `rust-toolchain.toml`.

```bash
cargo run --locked -- validate examples/stm32f4-workspace.yml
cargo run --locked -- plan examples/stm32f4-workspace.yml
cargo run --locked -- inspect examples/stm32f4-workspace.yml
```

Create the example product distribution:

```bash
cargo run --locked -- repack \
  --base examples/repack/base \
  --overlay examples/repack/overlay \
  --config examples/repack/product.yml \
  --output target/example/traction-control-unit.tar.gz
```

The command prints a JSON result containing output paths, archive and payload
digests, file count, and total payload bytes. Unless `--manifest` is supplied,
the sidecar is written to
`target/example/traction-control-unit.tar.gz.manifest.json`.

Verify the artifact independently with Python's standard library:

```bash
python scripts/verify_repack.py \
  target/example/traction-control-unit.tar.gz \
  target/example/traction-control-unit.tar.gz.manifest.json \
  --base examples/repack/base \
  --overlay examples/repack/overlay
```

Pass `--force` to replace existing regular archive and sidecar files. Without
it, the command fails safely rather than overwriting release output.

## Repack configuration

```yaml
schema_version: 1
product:
  name: traction-control-unit
  release: "2.4.0"
base_distribution:
  name: embedded-linux-sdk
  release: "12.3.1"
target: stm32mp157
executable_paths:
  - bin/start-product.sh
labels:
  configuration: vehicle-a
  release_channel: validated
```

Overlay files replace base files at exactly matching relative paths. Other
files from both trees remain in the result. Archive permissions come only from
`executable_paths`: declared files use `0755`, and all other payload files use
`0644`. This avoids host-permission differences in reproducible builds.

## TypeScript client

```bash
cd clients/typescript
npm ci
npm run build
npm test
npm audit --audit-level=high
```

## Design and evidence

- [Repack specification](docs/repack-specification.md)
- [Architecture and determinism](docs/architecture.md)
- [Verification strategy](docs/verification.md)
- [Scope and limitations](docs/limitations.md)

## Repository layout

```text
src/                  Rust parser, resolver, deterministic repacker, and CLI
examples/             Workspace manifests plus base/overlay/config fixtures
scripts/              Independent Python artifact verifier
clients/typescript/   TypeScript manifest client and tests
docs/                  Specification, architecture, verification, limitations
.github/workflows/    Linux CI and end-to-end reproducibility checks
```

## Local quality checks

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
```

See [Scope and limitations](docs/limitations.md) before applying the example to
a production release process.
