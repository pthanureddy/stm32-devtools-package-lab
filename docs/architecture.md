# Architecture

## Components

The existing manifest parser and dependency resolver remain independent of the
repack path. `src/main.rs` maps CLI arguments into `RepackRequest`, while
`src/commands.rs` owns terminal output. `src/repack.rs` provides the reusable
library API and contains four stages:

1. **Validate** — parse release metadata, canonicalize input roots, validate
   output locations, and enforce portable relative paths.
2. **Merge** — build a `BTreeMap` keyed by archive path. Overlay entries replace
   base entries deterministically and retain explicit layer provenance.
3. **Stage** — stream each selected source file to a temporary directory while
   calculating its SHA-256 and size. Explicit configuration, rather than host
   filesystem permissions, determines archive modes.
4. **Emit** — write fixed tar headers through a deterministic gzip encoder,
   synchronize temporary outputs, then persist the archive and sidecar.

The sorted map is the ordering boundary shared by the manifest and archive.
No wall-clock time, username, group, host path, or source-file timestamp enters
the output.

## Determinism model

Repacking the same bytes with semantically identical configuration and the same
tool version produces byte-identical archive and manifest outputs. Tar headers
use UID/GID 0, mtime 0, and fixed modes. The gzip header uses mtime 0, has no
source filename, and records the unknown operating-system identifier. CI builds
the fixture twice and compares both artifacts byte for byte.

## Failure model

Validation completes before source staging. Archive and sidecar data are fully
written to temporary files before existing outputs are removed for `--force`.
Errors carry an operation and path where applicable. A filesystem failure
between the two final renames can leave the archive without its sidecar; the
embedded manifest remains authoritative and the verifier detects divergence.
