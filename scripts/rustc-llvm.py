#!/usr/bin/env python3

"""Inspect or update the pinned Rust toolchains and their LLVM versions."""

from __future__ import annotations

import argparse
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
from enum import Enum
from http.client import HTTPResponse, IncompleteRead
from pathlib import Path
from typing import NamedTuple
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


SHA = r"[0-9a-f]{40}"
RELEASE = r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
LLVM_SOURCE_COMMENT = "# @llvm-project follows the nightly_rust_toolchains pin below."
MODULE = Path("MODULE.bazel").resolve()


class Channel(str, Enum):
    STABLE = "stable"
    BETA = "beta"
    NIGHTLY = "nightly"

    def __str__(self) -> str:
        return self.value


class LlvmPin(NamedTuple):
    commit: str
    version: str


class Candidate(NamedTuple):
    rust: str
    rustc_commit: str


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
            "@llvm//3rd_party/llvm-project/before23.x/patches:llvm-extra.patch",
            "@llvm//3rd_party/llvm-project/before23.x/patches:"
            "llvm-abi-breaking-checks.patch",
        ),
    ),
    22: Source(
        repository="rust-lang/llvm-project",
        commit="52ed14fcd56afc30f9cccd8ca8ce237c2eef7e04",
        version="22.1.8",
        integrity="sha256-J/MaZjkCPZDNdUwd5C2PnXfMDD3v83iW+IwTRsbD85Y=",
        patches=(
            "@llvm//3rd_party/llvm-project/22.x/patches:windows_link_and_genrule.patch",
            "@llvm//3rd_party/llvm-project/22.x/patches:"
            "llvm-bazel-blake3-windows-gnu.patch",
            "@llvm//3rd_party/llvm-project/before23.x/patches:llvm-extra.patch",
            "@llvm//3rd_party/llvm-project/before23.x/patches:"
            "llvm-abi-breaking-checks.patch",
        ),
    ),
}


def match_one(pattern: str, text: str, name: str) -> str:
    matches = list(re.finditer(pattern, text, re.MULTILINE))
    if len(matches) != 1:
        raise ValueError(f"expected exactly one {name}")
    return matches[0].group(1)


def read_pin(text: str) -> LlvmPin:
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
        r'^llvm\.version\(llvm_version = "(\d+\.\d+\.\d+)"\)$',
        text,
        "LLVM version",
    )
    return LlvmPin(commit, version)


def open_url(url: str) -> HTTPResponse:
    headers = {"User-Agent": "aya-rs/bpf-linker rustc LLVM updater"}
    if url.startswith("https://api.github.com/") and (token := os.getenv("GH_TOKEN")):
        headers |= {
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
        }
    return urlopen(Request(url, headers=headers), timeout=60)


def fetch(url: str, consume: Callable[[HTTPResponse], bytes]) -> bytes:
    attempt = 0
    while True:
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
        attempt += 1


def download(url: str) -> bytes:
    return fetch(url, lambda response: response.read())


def parse_release(toolchain: str) -> tuple[int, int, int]:
    match = re.fullmatch(RELEASE, toolchain)
    if match is None:
        raise ValueError("expected a full Rust release version")
    major, minor, patch = match.groups()
    return int(major), int(minor), int(patch)


def parse_dated_toolchain(toolchain: str) -> tuple[Channel, datetime.date]:
    channel, separator, value = toolchain.partition("-")
    if not separator:
        raise ValueError("expected a dated Rust prerelease toolchain")
    date = datetime.date.fromisoformat(value)
    if date.isoformat() != value:
        raise ValueError("expected a dated Rust prerelease toolchain")
    channel = Channel(channel)
    if channel == Channel.STABLE:
        raise ValueError("expected a full Rust release version")
    return channel, date


def selection_key(toolchain: str, channel: Channel) -> tuple[int, int, int]:
    if channel == Channel.STABLE:
        return parse_release(toolchain)
    actual_channel, date = parse_dated_toolchain(toolchain)
    if actual_channel != channel:
        raise ValueError(f"expected a dated Rust {channel}")
    return date.year, date.month, date.day


def resolve_commit(rust_commit: str) -> str:
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


def replace_llvm_source(
    text: str, replacements: tuple[tuple[str, str, str], ...]
) -> str:
    pattern = r"^llvm\.from_archive\(\n(?:.*\n)*?^\)$"
    matches = list(re.finditer(pattern, text, re.MULTILINE))
    if len(matches) != 1:
        raise ValueError("expected exactly one LLVM source block")
    match = matches[0]
    source = match.group()
    for pattern, value, name in replacements:
        source = replace_one(source, pattern, value, name)
    return text[: match.start()] + source + text[match.end() :]


def select_llvm(text: str, source: Source) -> str:
    updated = replace_one(
        text,
        "^" + re.escape(LLVM_SOURCE_COMMENT) + "$",
        f"# @llvm-project uses {source.repository} commit {source.commit}.",
        "LLVM source comment",
    )
    patches = "\n".join(f'        "{patch}",' for patch in source.patches)
    replacements = (
        (
            r'^    integrity = "[^"]+",$',
            f'    integrity = "{source.integrity}",',
            "LLVM integrity",
        ),
        (
            r'^    patches = (?:\[\]|'
            r'\["@llvm//[^\n]+"\]|\[\n'
            r'(?:        "@llvm//[^\n]+",\n)+    \]),$',
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
    updated = replace_one(
        updated,
        r'^llvm\.version\(llvm_version = "[^"]+"\)$',
        f'llvm.version(llvm_version = "{source.version}")',
        "LLVM version",
    )
    return replace_llvm_source(updated, replacements)


def configurations(text: str, pin: LlvmPin) -> dict[str, dict[str, str]]:
    configurations = {
        pin.version.partition(".")[0]: {
            "commit": pin.commit,
            "module": base64.b64encode(text.encode()).decode(),
        }
    }
    for version, source in COMPATIBILITY_SOURCES.items():
        configurations[str(version)] = {
            "commit": source.commit,
            "module": base64.b64encode(select_llvm(text, source).encode()).decode(),
        }
    return configurations


def supported_llvm(pin: LlvmPin) -> set[int]:
    return {int(pin.version.partition(".")[0]), *COMPATIBILITY_SOURCES}


def read_candidate(value: object, channel: Channel) -> Candidate:
    match value:
        case {"rust": str(rust), "rustc-commit": str(commit), **extra} if not extra:
            if not re.fullmatch(SHA, commit):
                raise ValueError(f"invalid Rust {channel} rustc commit")
            selection_key(rust, channel)
            return Candidate(rust, commit)
        case _:
            raise ValueError(f"invalid Rust {channel} candidate")


def read_candidates(text: str) -> dict[Channel, Candidate]:
    value = json.loads(text)
    if not isinstance(value, dict) or value.keys() != set(Channel):
        raise ValueError("expected stable, beta, and nightly Rust candidates")
    return {channel: read_candidate(value[channel], channel) for channel in Channel}


def buildozer(*commands: str) -> str:
    result = subprocess.run(
        [
            "bazel",
            "run",
            "--lockfile_mode=error",
            "@buildifier_prebuilt//:buildozer",
            "--",
            "-f",
            "-",
        ],
        input="\n".join(commands) + "\n",
        stdout=subprocess.PIPE,
        text=True,
    )
    # Buildozer returns 3 when an edit leaves the file unchanged.
    if result.returncode not in (0, 3):
        result.check_returncode()
    return result.stdout


def current_toolchains() -> dict[Channel, str]:
    versions = buildozer(
        *(f"print version|{MODULE}:{channel}_rust_toolchains" for channel in Channel)
    ).splitlines()
    if len(versions) != len(Channel):
        raise ValueError("expected one pin for each Rust channel")
    current = {}
    for channel, version in zip(Channel, versions):
        rust = version.replace("/", "-", 1)
        selection_key(rust, channel)
        current[channel] = rust
    return current


def llvm_selection(report: str, pin: LlvmPin) -> dict[str, str | int]:
    major = int(match_one(
        r"^LLVM version: (\d+)\.\d+\.\d+$", report, "rustc LLVM version"
    ))
    if major not in supported_llvm(pin):
        raise ValueError(f"rustc uses unsupported LLVM {major}")
    excluded = [
        f"llvm-{version}" for version in sorted(supported_llvm(pin)) if version != major
    ]
    if major != int(pin.version.partition(".")[0]):
        excluded.insert(0, "default")
    return {"llvm": major, "exclude-features": ",".join(excluded)}


def resolve_selection(
    current: str, channel: Channel, pin: LlvmPin, candidate: Candidate
) -> LlvmPin:
    if selection_key(candidate.rust, channel) < selection_key(current, channel):
        raise ValueError(f"Rust {channel} regressed from {current} to {candidate.rust}")
    commit = resolve_commit(candidate.rustc_commit)
    source = LlvmPin(commit, resolve_version(commit))
    if int(source.version.partition(".")[0]) not in supported_llvm(pin):
        raise ValueError(
            f"Rust {channel} {candidate.rust} uses unsupported LLVM {source.version}"
        )
    return source


def update_toolchains(
    pin: LlvmPin, candidates: dict[Channel, Candidate]
) -> dict[str, object] | None:
    current = current_toolchains()
    resolved = {
        channel: resolve_selection(previous, channel, pin, candidates[channel])
        for channel, previous in current.items()
    }
    commit, version = resolved[Channel.NIGHTLY]
    major = int(pin.version.partition(".")[0])
    if int(version.partition(".")[0]) != major:
        raise ValueError(
            f"Rust nightly {candidates[Channel.NIGHTLY].rust} uses unsupported LLVM {version}; "
            f"expected LLVM major {major}"
        )
    edits = [
        f"set version {json.dumps(candidate.rust.replace('-', '/', 1))}|"
        f"{MODULE}:{channel}_rust_toolchains"
        for channel, candidate in candidates.items()
        if candidate.rust != current[channel]
    ]
    if version != pin.version:
        edits.append(f"set llvm_version {json.dumps(version)}|{MODULE}:%llvm.version")
    if commit != pin.commit:
        edits.extend(
            f"set {attribute} {value}|{MODULE}:%llvm.from_archive"
            for attribute, value in (
                ("integrity", json.dumps(resolve_integrity(commit))),
                ("strip_prefix", json.dumps(f"llvm-project-{commit}")),
                (
                    "urls",
                    json.dumps([
                        f"https://github.com/rust-lang/llvm-project/archive/{commit}.tar.gz"
                    ]),
                ),
            )
        )
    if not edits:
        return None
    # Resolve and validate every channel before writing the declarations once.
    buildozer(*edits)
    return {
        "toolchains": {
            channel: {
                "rust": candidates[channel].rust,
                "llvm": int(value.version.partition(".")[0]),
            }
            for channel, value in resolved.items()
        },
        "llvm-commit": commit,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("configurations")
    commands.add_parser("toolchains")
    commands.add_parser("llvm")
    update = commands.add_parser("update")
    update.add_argument("candidates", type=read_candidates)
    arguments = parser.parse_args()

    if arguments.command == "toolchains":
        print(json.dumps(current_toolchains()))
        return
    text = MODULE.read_text()
    pin = read_pin(text)
    if arguments.command == "configurations":
        print(json.dumps(configurations(text, pin), separators=(",", ":"), sort_keys=True))
        return
    if arguments.command == "llvm":
        print(json.dumps(llvm_selection(sys.stdin.read(), pin)))
        return
    if updated := update_toolchains(pin, arguments.candidates):
        print(json.dumps(updated))


if __name__ == "__main__":
    main()
