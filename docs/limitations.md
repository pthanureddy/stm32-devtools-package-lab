# Scope and Limitations

- The repacker handles host-side regular files. Empty directories, symlinks,
  device nodes, sockets, named pipes, hard-link semantics, ACLs, and extended
  attributes are not represented.
- Overlay behavior is whole-file replacement. There is no line-level merge,
  deletion marker, patch application, dependency solver, or package-signing
  facility in the repack path.
- SHA-256 provides integrity evidence, not authenticity. Production release
  systems should sign the archive or manifest and protect signing keys outside
  this tool.
- Inputs are assumed not to be modified maliciously while a repack is running.
  Selected bytes are staged once to keep archive content and recorded digests
  consistent, but the tool is not a sandbox for adversarial concurrent writers.
- Reproducibility assumes the same input bytes, normalized configuration,
  product version of this CLI, and compression implementation.
- Archive and sidecar publication is not transactional as a unit; a filesystem
  failure between the two renames can leave an archive without its sidecar.
- With `--force`, the prior output pair is removed before the new pair is
  published, so a failed replacement may require recovery from another copy.
- The example models distribution assembly only. It does not claim firmware,
  RTOS, hardware, bootloader, railway-certification, cybersecurity-compliance,
  or target-device validation.
