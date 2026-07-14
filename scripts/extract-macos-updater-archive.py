#!/usr/bin/env python3
"""Preflight and extract one macOS updater app without following archive links."""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import errno
import gzip
import hashlib
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import sys
import tarfile
import tempfile
from typing import BinaryIO, Iterator, NamedTuple
import unicodedata


# These ceilings are deliberately generous for BatCave's small app bundle while
# keeping a malformed signed artifact from consuming unbounded CI memory or disk.
MAX_COMPRESSED_ARCHIVE_BYTES = 256 * 1024 * 1024
MAX_DECOMPRESSED_TAR_BYTES = 1152 * 1024 * 1024
MAX_MEMBER_COUNT = 50_000
MAX_PATH_DEPTH = 64
MAX_PATH_BYTES = 4_096
MAX_PATH_BOOKKEEPING_BYTES = 32 * 1024 * 1024
MAX_CANONICAL_PREFIXES = 100_000
MAX_FILE_BYTES = 256 * 1024 * 1024
MAX_EXPANDED_BYTES = 1024 * 1024 * 1024
COPY_CHUNK_BYTES = 1024 * 1024


class UnsafeArchive(ValueError):
    pass


class ArchiveRecord(NamedTuple):
    name: str
    parts: tuple[str, ...]
    kind: str
    size: int
    mode: int


class DescriptorState(NamedTuple):
    device: int
    inode: int
    mode: int
    size: int
    links: int
    modified_ns: int
    changed_ns: int


class DecompressedBudgetReader:
    """Limit bytes exposed to tarfile, including PAX/GNU metadata records."""

    def __init__(self, source: BinaryIO, limit: int) -> None:
        self.source = source
        self.limit = limit
        self.consumed = 0

    def read(self, size: int = -1) -> bytes:
        remaining = self.limit - self.consumed
        if remaining <= 0:
            raise UnsafeArchive(
                f"decompressed tar stream exceeds the {self.limit}-byte limit"
            )
        requested = remaining if size < 0 else min(size, remaining)
        chunk = self.source.read(requested)
        self.consumed += len(chunk)
        return chunk


def checked_parts(name: str) -> tuple[tuple[str, ...], int]:
    if not name or name.startswith("/") or PurePosixPath(name).is_absolute():
        raise UnsafeArchive(f"archive contains an absolute or empty path: {name!r}")
    if "\\" in name:
        raise UnsafeArchive(f"archive path contains a backslash: {name!r}")

    try:
        path_bytes = name.encode("utf-8")
    except UnicodeEncodeError as error:
        raise UnsafeArchive(f"archive path is not valid UTF-8: {name!r}") from error
    if len(path_bytes) > MAX_PATH_BYTES:
        raise UnsafeArchive(
            f"archive path exceeds the {MAX_PATH_BYTES}-byte limit: {name!r}"
        )

    parts = tuple(name.rstrip("/").split("/"))
    if not parts or any(part in {"", ".", ".."} for part in parts):
        raise UnsafeArchive(f"archive path is not canonical: {name!r}")
    if len(parts) > MAX_PATH_DEPTH:
        raise UnsafeArchive(
            f"archive path exceeds the {MAX_PATH_DEPTH}-component depth limit: {name!r}"
        )
    return parts, len(path_bytes)


def collision_key(parts: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(unicodedata.normalize("NFD", part).casefold() for part in parts)


def member_kind(member: tarfile.TarInfo) -> str:
    if member.isdir():
        return "directory"
    if member.isfile():
        return "file"
    if member.issym():
        raise UnsafeArchive(f"archive contains a symbolic link: {member.name!r}")
    if member.islnk():
        raise UnsafeArchive(f"archive contains a hard link: {member.name!r}")
    if member.ischr() or member.isblk():
        raise UnsafeArchive(f"archive contains a device entry: {member.name!r}")
    raise UnsafeArchive(f"archive contains an unsupported entry: {member.name!r}")


def read_next_member(archive: tarfile.TarFile) -> tarfile.TarInfo | None:
    member = archive.next()
    # Stream mode still appends yielded TarInfo values; the verifier retains its
    # bounded ArchiveRecord instead and never needs tarfile's member cache.
    archive.members.clear()
    return member


def preflight(
    archive: tarfile.TarFile, expected_app_name: str
) -> tuple[list[ArchiveRecord], str]:
    records: list[ArchiveRecord] = []
    roots: set[str] = set()
    canonical_paths: dict[
        tuple[str, ...], tuple[tuple[str, ...], str, bool]
    ] = {}
    expanded_bytes = 0
    path_bookkeeping_bytes = 0

    while (member := read_next_member(archive)) is not None:
        if len(records) >= MAX_MEMBER_COUNT:
            raise UnsafeArchive(
                f"archive exceeds the {MAX_MEMBER_COUNT}-member limit"
            )
        parts, path_bytes = checked_parts(member.name)
        path_bookkeeping_bytes += path_bytes
        if path_bookkeeping_bytes > MAX_PATH_BOOKKEEPING_BYTES:
            raise UnsafeArchive(
                "archive exceeds the "
                f"{MAX_PATH_BOOKKEEPING_BYTES}-byte path-bookkeeping limit"
            )
        roots.add(parts[0])

        nested_apps = [part for part in parts[1:] if part.casefold().endswith(".app")]
        if nested_apps:
            raise UnsafeArchive(f"archive contains an unexpected nested app: {member.name!r}")

        kind = member_kind(member)
        if member.size < 0:
            raise UnsafeArchive(f"archive entry has a negative size: {member.name!r}")
        if kind == "file":
            if member.size > MAX_FILE_BYTES:
                raise UnsafeArchive(
                    f"archive file exceeds the {MAX_FILE_BYTES}-byte limit: {member.name!r}"
                )
            if member.size > MAX_EXPANDED_BYTES - expanded_bytes:
                raise UnsafeArchive(
                    f"archive exceeds the {MAX_EXPANDED_BYTES}-byte expanded-size limit"
                )
            expanded_bytes += member.size

        for prefix_length in range(1, len(parts) + 1):
            prefix = parts[:prefix_length]
            key = collision_key(prefix)
            prefix_kind = kind if prefix_length == len(parts) else "directory"
            explicit = prefix_length == len(parts)
            existing = canonical_paths.get(key)
            if existing is None:
                if len(canonical_paths) >= MAX_CANONICAL_PREFIXES:
                    raise UnsafeArchive(
                        "archive exceeds the "
                        f"{MAX_CANONICAL_PREFIXES}-canonical-prefix limit"
                    )
                canonical_paths[key] = (prefix, prefix_kind, explicit)
                continue

            existing_prefix, existing_kind, existing_explicit = existing
            if existing_kind != prefix_kind:
                raise UnsafeArchive(
                    "archive path conflicts as both a file and directory: "
                    f"{member.name!r}"
                )
            if existing_prefix != prefix:
                raise UnsafeArchive(
                    "archive contains a filesystem-colliding path prefix: "
                    f"{member.name!r}"
                )
            if explicit and existing_explicit:
                raise UnsafeArchive(f"archive contains a duplicate path: {member.name!r}")
            if explicit:
                canonical_paths[key] = (prefix, prefix_kind, True)

        records.append(
            ArchiveRecord(member.name, parts, kind, member.size, member.mode & 0o777)
        )

    if roots != {expected_app_name}:
        found = ", ".join(sorted(roots)) if roots else "none"
        raise UnsafeArchive(
            f"archive must contain only the expected {expected_app_name!r} root; found: {found}"
        )

    root_record = canonical_paths[collision_key((expected_app_name,))]
    if root_record[1] != "directory":
        raise UnsafeArchive(f"expected app root is not a directory: {expected_app_name!r}")

    return records, expected_app_name


def descriptor_state(source: BinaryIO) -> DescriptorState:
    metadata = os.fstat(source.fileno())
    return DescriptorState(
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_size,
        metadata.st_nlink,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def copy_descriptor_pass(
    source: BinaryIO, size: int, output: BinaryIO | None
) -> bytes:
    source.seek(0)
    remaining = size
    digest = hashlib.sha256()
    while remaining:
        chunk = source.read(min(COPY_CHUNK_BYTES, remaining))
        if not chunk:
            raise UnsafeArchive("compressed archive ended before its descriptor size")
        digest.update(chunk)
        if output is not None:
            output.write(chunk)
        remaining -= len(chunk)
    return digest.digest()


def assert_descriptor_unchanged(
    source: BinaryIO, expected: DescriptorState, archive_path: Path
) -> None:
    if descriptor_state(source) != expected:
        raise UnsafeArchive(
            f"compressed archive descriptor changed while reading: {archive_path}"
        )


def snapshot_archive(archive_path: Path) -> BinaryIO:
    flags = os.O_RDONLY
    for flag_name in ("O_CLOEXEC", "O_NOFOLLOW", "O_NONBLOCK"):
        flags |= getattr(os, flag_name, 0)
    try:
        descriptor = os.open(os.fspath(archive_path), flags)
    except OSError as error:
        if error.errno == errno.ELOOP:
            raise UnsafeArchive(
                f"compressed archive path must not be a symbolic link: {archive_path}"
            ) from error
        raise

    try:
        source = os.fdopen(descriptor, "rb")
    except Exception:
        os.close(descriptor)
        raise

    snapshot = tempfile.TemporaryFile(mode="w+b")
    try:
        with source:
            initial = descriptor_state(source)
            if not stat.S_ISREG(initial.mode):
                raise UnsafeArchive(
                    f"compressed archive descriptor is not a regular file: {archive_path}"
                )
            if initial.size > MAX_COMPRESSED_ARCHIVE_BYTES:
                raise UnsafeArchive(
                    "compressed archive exceeds the "
                    f"{MAX_COMPRESSED_ARCHIVE_BYTES}-byte limit"
                )

            first_digest = copy_descriptor_pass(source, initial.size, snapshot)
            assert_descriptor_unchanged(source, initial, archive_path)
            second_digest = copy_descriptor_pass(source, initial.size, None)
            assert_descriptor_unchanged(source, initial, archive_path)
            if second_digest != first_digest:
                raise UnsafeArchive(
                    f"compressed archive contents changed while reading: {archive_path}"
                )
    except Exception:
        snapshot.close()
        raise

    snapshot.seek(0)
    return snapshot


@contextmanager
def streaming_tar(snapshot: BinaryIO) -> Iterator[tarfile.TarFile]:
    snapshot.seek(0)
    with gzip.GzipFile(fileobj=snapshot, mode="rb") as decompressed:
        budgeted = DecompressedBudgetReader(
            decompressed, MAX_DECOMPRESSED_TAR_BYTES
        )
        with tarfile.open(fileobj=budgeted, mode="r|") as archive:
            yield archive


def copy_member(
    source: BinaryIO, output: BinaryIO, declared_size: int, remaining_budget: int
) -> int:
    if declared_size > remaining_budget:
        raise UnsafeArchive("archive extraction exceeded its expanded-size budget")

    remaining = declared_size
    while remaining:
        chunk = source.read(min(COPY_CHUNK_BYTES, remaining))
        if not chunk:
            raise UnsafeArchive("archive file ended before its declared size")
        if len(chunk) > remaining:
            raise UnsafeArchive("archive file exceeded its declared size")
        output.write(chunk)
        remaining -= len(chunk)

    if source.read(1):
        raise UnsafeArchive("archive file exceeded its declared size")
    return declared_size


def verify_member(member: tarfile.TarInfo, record: ArchiveRecord) -> None:
    parts, _ = checked_parts(member.name)
    if (
        member.name != record.name
        or parts != record.parts
        or member_kind(member) != record.kind
        or member.size != record.size
        or member.mode & 0o777 != record.mode
    ):
        raise UnsafeArchive("archive contents changed between preflight and extraction")


def extract(archive_path: Path, destination: Path, expected_app_name: str) -> Path:
    if destination.exists():
        raise UnsafeArchive(f"destination already exists: {destination}")

    with snapshot_archive(archive_path) as snapshot:
        with streaming_tar(snapshot) as archive:
            records, app_root = preflight(archive, expected_app_name)

        destination.mkdir(mode=0o700, parents=True)
        try:
            directories = [record for record in records if record.kind == "directory"]
            for record in sorted(directories, key=lambda item: len(item.parts)):
                destination.joinpath(*record.parts).mkdir(
                    mode=0o700, parents=True, exist_ok=True
                )

            remaining_expanded_bytes = MAX_EXPANDED_BYTES
            with streaming_tar(snapshot) as archive:
                for record in records:
                    member = read_next_member(archive)
                    if member is None:
                        raise UnsafeArchive(
                            "archive ended between preflight and extraction"
                        )
                    verify_member(member, record)
                    if record.kind != "file":
                        continue
                    target = destination.joinpath(*record.parts)
                    target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                    source = archive.extractfile(member)
                    if source is None:
                        raise UnsafeArchive(f"archive file has no data: {member.name!r}")
                    with source, target.open("xb") as output:
                        copied = copy_member(
                            source, output, record.size, remaining_expanded_bytes
                        )
                    remaining_expanded_bytes -= copied
                    target.chmod(record.mode)

                if read_next_member(archive) is not None:
                    raise UnsafeArchive(
                        "archive gained entries between preflight and extraction"
                    )

            for record in sorted(
                directories, key=lambda item: len(item.parts), reverse=True
            ):
                destination.joinpath(*record.parts).chmod(record.mode)
        except Exception:
            shutil.rmtree(destination, ignore_errors=True)
            raise

    return destination / app_root


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("archive", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--expected-app-name", required=True)
    args = parser.parse_args()

    try:
        app = extract(args.archive, args.destination, args.expected_app_name)
    except (OSError, tarfile.TarError, UnsafeArchive) as error:
        print(f"Unsafe macOS updater archive: {error}", file=sys.stderr)
        return 1

    print(app)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
