# Distribution Repack Specification

## Purpose

`stm32pkg repack` creates a product distribution by applying a complete
product-specific overlay directory to an existing base directory. A matching
overlay file replaces the base file at the same relative path; otherwise files
from both layers are retained. The command creates a deterministic `tar.gz`, an
embedded `PACKAGE-MANIFEST.json`, and a byte-identical sidecar manifest.

This is a host-side packaging operation. It does not compile, sign, flash, or
execute firmware.

## Inputs

- `--base`: readable directory containing an existing distribution.
- `--overlay`: readable directory containing product additions/replacements.
- `--config`: YAML or JSON configuration with an explicit schema version 1;
  unknown fields are rejected so misspelled controls cannot be ignored.
- `--output`: destination archive, conventionally ending in `.tar.gz`.
- `--manifest`: optional sidecar path. The default is
  `<output>.manifest.json`.
- `--force`: explicitly permits replacement of existing regular output files.

The configuration records product and base-distribution names and semantic
releases, the target, optional labels, and paths that must receive mode `0755`.
Every other payload file receives mode `0644`.

## Merge and manifest rules

1. Walk the base and overlay without following links.
2. Convert every regular file to a portable, UTF-8, forward-slash path.
3. Insert base files, then replace same-path entries with overlay files.
4. Reject a merged tree where one regular-file path is an ancestor of another.
5. Stage and hash files in sorted path order.
6. Record path, source layer, byte size, SHA-256, and archive mode for each file.
7. Compute `payload.sha256` over the canonical sorted record stream:
   `path NUL layer NUL size NUL sha256 NUL mode NUL LF`.
8. Serialize the manifest as pretty JSON with a trailing newline.
9. Add payload files and the embedded manifest to the tar in lexical path order.

The configuration digest is computed from its normalized JSON representation,
including sorted executable paths, so YAML formatting and semantically
irrelevant executable-list ordering do not alter the package identity.

## Safety invariants

- Absolute paths, `.`/`..`, empty segments, backslashes, colons, control
  characters, non-UTF-8 archive names, and names above the canonical 100-byte
  tar-header limit are rejected.
- Symlinks and non-regular special files are rejected in either input tree.
- Base and overlay trees may not be equal, nested, or otherwise overlap.
- Archive and sidecar outputs may not be placed inside either input tree.
- The embedded `PACKAGE-MANIFEST.json` path and its case-insensitive descendants
  are reserved and cannot be supplied by either payload layer.
- Archive and sidecar outputs may not replace the configuration file.
- Existing outputs are preserved unless `--force` is supplied.
- Payload bytes are staged while hashing, then archived from staging. The hash
  therefore describes the exact bytes written rather than a second source read.
