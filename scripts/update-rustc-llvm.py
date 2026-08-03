#!/usr/bin/env python3

"""Inspect or update the Rust nightly and LLVM pinned by MODULE.bazel."""

from __future__ import annotations

import base64
import datetime
import hashlib
import json
import re
import sys
from http.client import HTTPResponse, IncompleteRead
from pathlib import Path
from typing import Any

from nightly import Pin, SHA, download, fetch, main, match_one, replace_one


def read_pin(text: str) -> Pin:
    date = match_one(r"^# nightly-(\d{4}-\d{2}-\d{2})\.$", text, "nightly pin")
    datetime.date.fromisoformat(date)
    commit = match_one(
        rf'^\s*urls = \["https://github\.com/rust-lang/'
        rf'llvm-project/archive/({SHA})\.tar\.gz"\],\s*$',
        text,
        "LLVM archive",
    )
    prefix = match_one(
        rf'^\s*strip_prefix = "llvm-project-({SHA})",\s*$',
        text,
        "LLVM strip prefix",
    )
    if prefix != commit:
        raise ValueError("LLVM archive and strip prefix do not match")
    version = match_one(
        r'^llvm_source\.version\(llvm_version = "(\d+\.\d+\.\d+)"\)$',
        text,
        "LLVM version",
    )
    return Pin(f"nightly-{date}", {"commit": commit, "version": version})


def resolve_commit(manifest: dict[str, Any]) -> str:
    rust_commit = manifest["pkg"]["rustc"]["git_commit_hash"]
    if not isinstance(rust_commit, str) or not re.fullmatch(SHA, rust_commit):
        raise ValueError("nightly manifest contains an invalid rustc commit")
    source = json.loads(
        download(
            "https://api.github.com/repos/rust-lang/rust/contents/"
            f"src/llvm-project?ref={rust_commit}"
        )
    )
    if (
        source.get("submodule_git_url")
        != "https://github.com/rust-lang/llvm-project.git"
    ):
        raise ValueError("rustc does not reference the expected LLVM repository")
    commit = source["sha"]
    if not isinstance(commit, str) or not re.fullmatch(SHA, commit):
        raise ValueError("rustc references an invalid LLVM commit")
    return commit


def resolve_version(commit: str) -> str:
    source = download(
        f"https://raw.githubusercontent.com/rust-lang/llvm-project/{commit}/"
        "cmake/Modules/LLVMVersion.cmake"
    ).decode()
    return ".".join(
        match_one(
            rf"^\s*set\(LLVM_VERSION_{part}\s+(\d+)\)\s*$",
            source,
            f"LLVM {part.lower()} version",
        )
        for part in ("MAJOR", "MINOR", "PATCH")
    )


def resolve_integrity(commit: str) -> str:
    def digest_archive(archive: HTTPResponse) -> bytes:
        digest = hashlib.sha256()
        while chunk := archive.read(1024 * 1024):
            digest.update(chunk)
        if archive.length:
            raise IncompleteRead(b"", archive.length)
        return digest.digest()

    digest = fetch(
        f"https://github.com/rust-lang/llvm-project/archive/{commit}.tar.gz",
        digest_archive,
    )
    return "sha256-" + base64.b64encode(digest).decode()


def update_module(
    text: str, current: Pin, nightly: str, manifest: dict[str, Any]
) -> str | None:
    commit = resolve_commit(manifest)
    if (nightly, commit) == (current.nightly, current.metadata["commit"]):
        return None
    updated = replace_one(
        text, r"^# nightly-\d{4}-\d{2}-\d{2}\.$", f"# {nightly}.", "nightly pin"
    )
    if commit == current.metadata["commit"]:
        return updated

    version = resolve_version(commit)
    if version.partition(".")[0] != current.metadata["version"].partition(".")[0]:
        print(
            f"::notice::Rust nightly uses unsupported LLVM {version}; "
            f"keeping LLVM {current.metadata['version']}.",
            file=sys.stderr,
        )
        return None
    replacements = (
        (
            r'^llvm_source\.version\(llvm_version = "[^"]+"\)$',
            f'llvm_source.version(llvm_version = "{version}")',
            "LLVM version",
        ),
        (
            r'^    integrity = "[^"]+",$',
            f'    integrity = "{resolve_integrity(commit)}",',
            "LLVM integrity",
        ),
        (
            rf'^    strip_prefix = "llvm-project-{SHA}",$',
            f'    strip_prefix = "llvm-project-{commit}",',
            "LLVM strip prefix",
        ),
        (
            rf'^    urls = \["https://github\.com/rust-lang/'
            rf'llvm-project/archive/{SHA}\.tar\.gz"\],$',
            f'    urls = ["https://github.com/rust-lang/llvm-project/archive/{commit}.tar.gz"],',
            "LLVM archive",
        ),
    )
    for pattern, value, name in replacements:
        updated = replace_one(updated, pattern, value, name)
    return updated


if __name__ == "__main__":
    sys.exit(
        main(Path(__file__).resolve().parent.parent / "MODULE.bazel", read_pin, update_module)
    )
