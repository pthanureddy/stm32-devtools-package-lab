# Verification Strategy

## Rust tests

The repack module has focused tests for:

- Unix and Windows-style absolute paths, parent traversal, and backslashes;
- portable path acceptance and configuration schema validation;
- required schema version, unknown configuration fields, and canonical
  executable-list ordering;
- duplicate or missing executable declarations;
- unsupported configuration formats;
- overlay precedence and recorded source provenance;
- sorted manifest records and embedded/sidecar equality;
- byte-for-byte archive reproducibility;
- deterministic executable and regular-file modes;
- safe refusal and explicit replacement of existing outputs;
- rejection of outputs inside input roots;
- protection of the configuration file and reserved embedded-manifest path;
- file/directory collisions; and
- symlink rejection on Unix CI hosts.

These complement the existing package-manifest and dependency-order tests.

## End-to-end verification

GitHub Actions repacks the checked-in base and overlay fixtures twice. It uses
`cmp` on both archives and sidecars, then runs
`scripts/verify_repack.py`. The standard-library-only verifier independently:

- rejects unsafe, duplicate, linked, unsorted, or unexpected tar members;
- compares the embedded manifest to the sidecar byte for byte;
- recomputes each payload SHA-256 and size from archive bytes;
- verifies fixed tar metadata and configured modes;
- recomputes the canonical payload digest and summary totals; and
- rebuilds the expected base-plus-overlay inventory to check precedence.

The CI Rust job separately runs `cargo fmt --check`, Clippy with warnings denied,
and all tests. The TypeScript job builds, tests, and fails on high-severity npm
audit findings.

## Manual exercise

```bash
cargo run -- repack \
  --base examples/repack/base \
  --overlay examples/repack/overlay \
  --config examples/repack/product.yml \
  --output target/example/traction-control-unit.tar.gz

python scripts/verify_repack.py \
  target/example/traction-control-unit.tar.gz \
  target/example/traction-control-unit.tar.gz.manifest.json \
  --base examples/repack/base \
  --overlay examples/repack/overlay
```
