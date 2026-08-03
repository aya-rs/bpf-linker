#!/usr/bin/env python3

"""Inspect or update the Rust nightly and LLVM pinned by MODULE.bazel."""

from __future__ import annotations

import argparse
import base64
import datetime
import hashlib
import itertools
import json
import os
import re
import sys
import time
import tomllib
from collections.abc import Callable
from http.client import HTTPResponse, IncompleteRead
from typing import NamedTuple
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


SHA = r"[0-9a-f]{40}"


class Pin(NamedTuple):
    nightly: str
    commit: str
    version: str


def match_one(pattern: str, text: str, name: str) -> str:
    matches = list(re.finditer(pattern, text, re.MULTILINE))
    if len(matches) != 1:
        raise ValueError(f"expected exactly one {name}")
    return matches[0].group(1)


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
    return Pin(f"nightly-{date}", commit, version)


def open_url(url: str) -> HTTPResponse:
    headers = {"User-Agent": "aya-rs/bpf-linker rustc LLVM updater"}
    if url.startswith("https://api.github.com/") and (token := os.getenv("GH_TOKEN")):
        headers |= {
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
        }
    return urlopen(Request(url, headers=headers), timeout=60)


def fetch(url: str, consume: Callable[[HTTPResponse], bytes]) -> bytes:
    for attempt in itertools.count():
        try:
            with open_url(url) as response:
                return consume(response)
        except HTTPError as error:
            if attempt == 5 or not (
                error.code in (408, 429) or 500 <= error.code < 600
            ):
                raise
        except (URLError, TimeoutError, ConnectionError, IncompleteRead):
            if attempt == 5:
                raise
        time.sleep(2**attempt)


def download(url: str) -> bytes:
    return fetch(url, lambda response: response.read())


def resolve_commit(nightly: str) -> str:
    date = nightly.removeprefix("nightly-")
    manifest = tomllib.loads(
        download(
            f"https://static.rust-lang.org/dist/{date}/channel-rust-nightly.toml"
        ).decode()
    )
    if manifest.get("date") != date:
        raise ValueError("nightly manifest does not match the pinned date")
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


def replace_one(text: str, pattern: str, value: str, name: str) -> str:
    result, count = re.subn(pattern, lambda _: value, text, flags=re.MULTILINE)
    if count != 1:
        raise ValueError(f"expected exactly one {name}")
    return result


def update_module(text: str, current: Pin, nightly: str, commit: str) -> str | None:
    if (nightly, commit) == (current.nightly, current.commit):
        return None

    updated = replace_one(
        text, r"^# nightly-\d{4}-\d{2}-\d{2}\.$", f"# {nightly}.", "nightly pin"
    )
    if commit == current.commit:
        return updated

    version = resolve_version(commit)
    major = current.version.partition(".")[0]
    if version.partition(".")[0] != major:
        raise ValueError(
            f"Rust nightly {nightly} uses unsupported LLVM {version}; "
            f"expected LLVM major {major}"
        )
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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("current")
    update = commands.add_parser("update")
    update.add_argument("nightly")
    arguments = parser.parse_args()

    text = sys.stdin.read()
    pin = read_pin(text)
    if arguments.command == "current":
        print(pin.nightly, pin.commit)
        return

    nightly = arguments.nightly
    match = re.fullmatch(r"nightly-(\d{4}-\d{2}-\d{2})", nightly)
    if match is None:
        raise ValueError("expected a dated Rust nightly")
    datetime.date.fromisoformat(match.group(1))
    if nightly < pin.nightly:
        raise ValueError(f"Rust nightly regressed from {pin.nightly} to {nightly}")

    if updated := update_module(text, pin, nightly, resolve_commit(nightly)):
        read_pin(updated)
        sys.stdout.write(updated)


if __name__ == "__main__":
    main()
