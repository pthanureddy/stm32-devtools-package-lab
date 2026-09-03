#!/usr/bin/env python3
"""Independently verify a stm32pkg deterministic repack artifact."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import tarfile
from pathlib import Path, PurePosixPath
from typing import Any


EMBEDDED_MANIFEST = "PACKAGE-MANIFEST.json"


class VerificationError(RuntimeError):
    """Raised when an artifact violates the repack contract."""


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def safe_member_name(name: str) -> None:
    path = PurePosixPath(name)
    if (
        not name
        or path.is_absolute()
        or "\\" in name
        or any(part in {"", ".", ".."} for part in path.parts)
        or any(":" in part or any(ord(char) < 32 for char in part) for part in path.parts)
    ):
        raise VerificationError(f"unsafe archive member path: {name!r}")


def canonical_payload_digest(records: list[dict[str, Any]]) -> str:
    digest = hashlib.sha256()
    for record in records:
        fields = (
            record["path"],
            record["layer"],
            str(record["size"]),
            record["sha256"],
            record["mode"],
        )
        for field in fields:
            digest.update(field.encode("utf-8"))
            digest.update(b"\0")
        digest.update(b"\n")
    return digest.hexdigest()


def expected_overlay(base: Path, overlay: Path) -> dict[str, tuple[bytes, str]]:
    merged: dict[str, tuple[bytes, str]] = {}
    for root, layer in ((base, "base"), (overlay, "overlay")):
        if not root.is_dir():
            raise VerificationError(f"expected fixture directory is missing: {root}")
        for path in sorted(root.rglob("*")):
            if path.is_symlink():
                raise VerificationError(f"fixture contains a symlink: {path}")
            if path.is_dir():
                continue
            if not path.is_file():
                raise VerificationError(f"fixture contains a special file: {path}")
            relative = path.relative_to(root).as_posix()
            safe_member_name(relative)
            merged[relative] = (path.read_bytes(), layer)
    return merged


def verify(
    archive_path: Path,
    sidecar_path: Path,
    base: Path | None,
    overlay: Path | None,
) -> dict[str, Any]:
    raw_archive = archive_path.read_bytes()
    if len(raw_archive) < 10 or raw_archive[:2] != b"\x1f\x8b":
        raise VerificationError("archive is not a gzip stream")
    if raw_archive[4:8] != b"\0\0\0\0":
        raise VerificationError("gzip mtime is not zero")
    if raw_archive[9] != 255:
        raise VerificationError("gzip operating-system byte is not deterministic (255)")

    sidecar_bytes = sidecar_path.read_bytes()
    with tarfile.open(archive_path, mode="r:gz") as archive:
        members = archive.getmembers()
        names = [member.name for member in members]
        if names != sorted(names):
            raise VerificationError("tar members are not in lexical order")
        if len(names) != len(set(names)):
            raise VerificationError("tar contains duplicate member names")

        content: dict[str, bytes] = {}
        member_modes: dict[str, int] = {}
        for member in members:
            safe_member_name(member.name)
            if not member.isfile():
                raise VerificationError(f"non-regular tar member: {member.name}")
            if member.uid != 0 or member.gid != 0 or member.mtime != 0:
                raise VerificationError(f"non-deterministic tar metadata: {member.name}")
            extracted = archive.extractfile(member)
            if extracted is None:
                raise VerificationError(f"cannot read tar member: {member.name}")
            content[member.name] = extracted.read()
            member_modes[member.name] = member.mode

    embedded_bytes = content.get(EMBEDDED_MANIFEST)
    if embedded_bytes is None:
        raise VerificationError("embedded manifest is missing")
    if embedded_bytes != sidecar_bytes:
        raise VerificationError("embedded and sidecar manifests differ")
    if member_modes[EMBEDDED_MANIFEST] != 0o644:
        raise VerificationError("embedded manifest mode is not 0644")

    manifest = json.loads(sidecar_bytes)
    if manifest.get("schema_version") != 1 or manifest.get("archive_format") != "tar.gz":
        raise VerificationError("unsupported manifest schema or archive format")
    records = manifest.get("files")
    if not isinstance(records, list):
        raise VerificationError("manifest files must be a list")
    record_paths = [record.get("path") for record in records]
    if record_paths != sorted(record_paths) or len(record_paths) != len(set(record_paths)):
        raise VerificationError("manifest file records are not sorted and unique")

    expected_members = set(record_paths) | {EMBEDDED_MANIFEST}
    if set(content) != expected_members:
        missing = sorted(expected_members - set(content))
        extra = sorted(set(content) - expected_members)
        raise VerificationError(f"archive/manifest inventory mismatch; missing={missing}, extra={extra}")

    total_size = 0
    for record in records:
        path = record["path"]
        safe_member_name(path)
        payload = content[path]
        if record.get("layer") not in {"base", "overlay"}:
            raise VerificationError(f"invalid layer for {path}")
        if record.get("size") != len(payload):
            raise VerificationError(f"size mismatch for {path}")
        if record.get("sha256") != sha256(payload):
            raise VerificationError(f"SHA-256 mismatch for {path}")
        if record.get("mode") not in {"0644", "0755"}:
            raise VerificationError(f"unexpected configured mode for {path}")
        if member_modes[path] != int(record["mode"], 8):
            raise VerificationError(f"tar/manifest mode mismatch for {path}")
        total_size += len(payload)

    summary = manifest.get("payload", {})
    if summary.get("file_count") != len(records) or summary.get("total_size") != total_size:
        raise VerificationError("payload count or total size does not match records")
    expected_digest = canonical_payload_digest(records)
    if summary.get("sha256") != expected_digest:
        raise VerificationError("canonical payload digest does not match records")

    if (base is None) != (overlay is None):
        raise VerificationError("--base and --overlay must be supplied together")
    if base is not None and overlay is not None:
        expected = expected_overlay(base, overlay)
        if set(expected) != set(record_paths):
            raise VerificationError("fixture merge inventory does not match manifest")
        record_by_path = {record["path"]: record for record in records}
        for path, (expected_bytes, expected_layer) in expected.items():
            if content[path] != expected_bytes:
                raise VerificationError(f"overlay merge content mismatch for {path}")
            if record_by_path[path]["layer"] != expected_layer:
                raise VerificationError(f"overlay provenance mismatch for {path}")

    return {
        "status": "PASS",
        "archive_sha256": sha256(raw_archive),
        "payload_sha256": expected_digest,
        "file_count": len(records),
        "total_size": total_size,
        "product": manifest.get("product"),
        "base_distribution": manifest.get("base_distribution"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--base", type=Path)
    parser.add_argument("--overlay", type=Path)
    args = parser.parse_args()

    try:
        result = verify(args.archive, args.manifest, args.base, args.overlay)
    except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError, tarfile.TarError) as error:
        print(f"verification failed: {error}", file=sys.stderr)
        return 1
    except VerificationError as error:
        print(f"verification failed: {error}", file=sys.stderr)
        return 1

    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
