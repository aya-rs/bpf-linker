#!/usr/bin/env python3

"""Inspect or update the Rust nightly and LLVM pinned by MODULE.bazel."""

from __future__ import annotations

import argparse
import base64
import datetime
import hashlib
import json
import os
import re
import sys
import time
import tomllib
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


class Source(NamedTuple):
    repository: str
    commit: str
    version: str
    integrity: str
    patches: tuple[str, ...]


COMPATIBILITY_SOURCES = {
    21: Source(
        repository="aya-rs/llvm-project",
        commit="00d23d10dc48c6bb9d57ba96d4a748d85d77d0c7",
        version="21.1.8",
        integrity="sha256-VDptNFG5E4/tVGR8bQlfrBXsmjkrxOhpt2hapPu0T6U=",
        patches=(
            "@llvm//3rd_party/llvm-project/21.x/patches:llvm-bazel9.patch",
            "@llvm//3rd_party/llvm-project/21.x/patches:windows_link_and_genrule.patch",
            "@llvm//3rd_party/llvm-project/21.x/patches:"
            "llvm-bazel-blake3-windows-gnu.patch",
            "@llvm//3rd_party/llvm-project/x.x/patches:llvm-extra.patch",
            "@llvm//3rd_party/llvm-project/x.x/patches:llvm-abi-breaking-checks.patch",
        ),
    ),
}


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
    manifest = tomllib.loads(
        download("https://static.rust-lang.org/dist/channel-rust-nightly.toml").decode()
    )
    date = datetime.date.fromisoformat(str(manifest["date"])).isoformat()
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
    return f"nightly-{date}", commit


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


def select_llvm(text: str, source: Source) -> str:
    updated = replace_one(
        text,
        r"^# @llvm-project uses the rust-lang/llvm-project commit referenced by\n"
        r"# nightly-\d{4}-\d{2}-\d{2}\.$",
        f"# @llvm-project uses {source.repository} commit {source.commit}.",
        "LLVM source comment",
    )
    patches = "\n".join(f'        "{patch}",' for patch in source.patches)
    replacements = (
        (
            r'^llvm_source\.version\(llvm_version = "[^"]+"\)$',
            f'llvm_source.version(llvm_version = "{source.version}")',
            "LLVM version",
        ),
        (
            r'^    integrity = "[^"]+",$',
            f'    integrity = "{source.integrity}",',
            "LLVM integrity",
        ),
        (
            r'^    patches = \[\n(?:        "@llvm//[^\n]+",\n)+    \],$',
            f"    patches = [\n{patches}\n    ],",
            "LLVM patches",
        ),
        (
            rf'^    strip_prefix = "llvm-project-{SHA}",$',
            f'    strip_prefix = "llvm-project-{source.commit}",',
            "LLVM strip prefix",
        ),
        (
            rf'^    urls = \["https://github\.com/[^/]+/llvm-project/archive/'
            rf'{SHA}\.tar\.gz"\],$',
            f'    urls = ["https://github.com/{source.repository}/archive/'
            f'{source.commit}.tar.gz"],',
            "LLVM archive",
        ),
    )
    for pattern, value, name in replacements:
        updated = replace_one(updated, pattern, value, name)
    return updated


def write_outputs(pin: Pin, module: str | None = None) -> None:
    outputs = {
        "rust-nightly": pin.nightly,
        "llvm-commit": pin.commit,
        "llvm-version": pin.version,
    }
    if module is not None:
        outputs["module"] = base64.b64encode(module.encode()).decode()
    rendered = "".join(f"{key}={value}\n" for key, value in outputs.items())
    print(rendered, end="")
    if output := os.getenv("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as destination:
            destination.write(rendered)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--update",
        action="store_true",
        help="update MODULE.bazel to the latest supported Rust nightly",
    )
    mode.add_argument(
        "--llvm-version",
        type=int,
        choices=COMPATIBILITY_SOURCES,
        help="render MODULE.bazel for a supported compatibility LLVM version",
    )
    args = parser.parse_args()
    try:
        text = MODULE.read_text(encoding="utf-8")
        pin = read_pin(text)
        if args.llvm_version is not None:
            source = COMPATIBILITY_SOURCES[args.llvm_version]
            write_outputs(
                Pin(pin.nightly, source.commit, source.version),
                select_llvm(text, source),
            )
        elif not args.update:
            write_outputs(pin)
        elif updated := update_module(text, pin, *resolve_latest()):
            candidate = read_pin(updated)
            MODULE.write_text(updated, encoding="utf-8")
            write_outputs(candidate, updated)
        return 0
    except (IncompleteRead, KeyError, OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
