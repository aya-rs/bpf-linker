#!/usr/bin/env python3

"""Inspect or update the Rust nightly and LLVM pinned by MODULE.bazel."""

from __future__ import annotations

import base64
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from collections.abc import Callable
from http.client import HTTPResponse, IncompleteRead
from pathlib import Path
from typing import NamedTuple
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


MODULE = Path(__file__).resolve().parent.parent / "MODULE.bazel"
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


def open_url(url: str):
    headers = {"User-Agent": "aya-rs/bpf-linker rustc LLVM updater"}
    if url.startswith("https://api.github.com/") and (token := os.getenv("GH_TOKEN")):
        headers |= {
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
        }
    return urlopen(Request(url, headers=headers), timeout=60)


def fetch(url: str, consume: Callable[[HTTPResponse], bytes]) -> bytes:
    for attempt in range(6):
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
    raise AssertionError("unreachable")


def download(url: str) -> bytes:
    return fetch(url, lambda response: response.read())


def resolve_latest() -> tuple[str, str]:
    result = subprocess.run(
        [sys.executable, os.environ["RUST_NIGHTLY_UPDATER"], "latest"],
        capture_output=True,
        text=True,
        check=True,
    )
    latest = json.loads(result.stdout)
    if not isinstance(latest, dict):
        raise ValueError("nightly updater returned an invalid result")
    nightly = latest.get("nightly")
    if not isinstance(nightly, str):
        raise ValueError("nightly updater returned an invalid nightly")
    match = re.fullmatch(r"nightly-(\d{4}-\d{2}-\d{2})", nightly)
    if (
        match is None
        or datetime.date.fromisoformat(match.group(1)).isoformat() != match.group(1)
    ):
        raise ValueError("nightly updater returned an invalid nightly")
    rust_commit = latest.get("rust_commit")
    if not isinstance(rust_commit, str) or not re.fullmatch(SHA, rust_commit):
        raise ValueError("nightly updater returned an invalid rustc commit")
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
    return nightly, commit


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
    if version.partition(".")[0] != current.version.partition(".")[0]:
        print(
            f"::notice::Rust nightly uses unsupported LLVM {version}; "
            f"keeping LLVM {current.version}.",
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


def write_outputs(pin: Pin) -> None:
    outputs = {
        "rust-nightly": pin.nightly,
        "llvm-commit": pin.commit,
        "llvm-version": pin.version,
    }
    rendered = "".join(f"{key}={value}\n" for key, value in outputs.items())
    print(rendered, end="")
    if output := os.getenv("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as destination:
            destination.write(rendered)


def main() -> None:
    arguments = sys.argv[1:]
    if arguments not in ([], ["update"]):
        raise SystemExit(f"usage: {sys.argv[0]} [update]")

    text = MODULE.read_text(encoding="utf-8")
    pin = read_pin(text)
    if not arguments:
        write_outputs(pin)
        return

    nightly, commit = resolve_latest()
    if nightly < pin.nightly:
        raise ValueError(
            "nightly manifest regressed from "
            f"{pin.nightly.removeprefix('nightly-')} to "
            f"{nightly.removeprefix('nightly-')}"
        )
    if updated := update_module(text, pin, nightly, commit):
        candidate = read_pin(updated)
        MODULE.write_text(updated, encoding="utf-8")
        write_outputs(candidate)


if __name__ == "__main__":
    main()
