#!/usr/bin/env python3

"""Prepare a musl-compatible sysroot using Alpine minirootfs archives."""

from __future__ import annotations

import argparse
import logging
import os
import re
import shutil
import ssl
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Dict, List, Tuple
from urllib.error import HTTPError, URLError
from urllib.request import urlopen


LOGGER = logging.getLogger("prepare-musl-sysroot")
MINIROOTFS_PATTERN = "alpine-minirootfs-{version}-{arch}.tar.gz"


def build_tls_contexts() -> List[ssl.SSLContext]:
    """Return contexts ordered by preference to maximize compatibility."""
    contexts: List[ssl.SSLContext] = [ssl.create_default_context()]
    # Some environments block TLS 1.3 traffic, which manifests as an OpenSSL
    # "record layer failure". Retry with a TLS 1.2-only context when possible.
    tls_version = getattr(ssl, "TLSVersion", None)
    if tls_version is not None and getattr(ssl, "HAS_TLSv1_3", False):
        tls12_context = ssl.create_default_context()
        tls12_context.minimum_version = tls_version.TLSv1_2
        tls12_context.maximum_version = tls_version.TLSv1_2
        contexts.append(tls12_context)
    return contexts


# Precompute TLS contexts once so downloads can reuse the same objects.
TLS_CONTEXTS = build_tls_contexts()


def download(url: str, *, copy_to: Path | None = None) -> bytes:
    for idx, context in enumerate(TLS_CONTEXTS):
        try:
            with urlopen(url, context=context) as response:
                if copy_to is None:
                    return response.read()
                with copy_to.open("wb") as fp:
                    shutil.copyfileobj(response, fp)
                return b""
        except HTTPError as err:
            raise RuntimeError(
                f"Failed to download {url}: HTTP {err.code}"
            ) from err
        except URLError as err:
            raise RuntimeError(
                f"Failed to download {url}: {err.reason}"
            ) from err
        except ssl.SSLError as err:
            if idx + 1 < len(TLS_CONTEXTS):
                LOGGER.info(
                    "TLS handshake failed (%s), retrying with TLS 1.2 fallback", err
                )
                continue
            raise RuntimeError(
                f"TLS handshake failed while downloading {url}: {err}"
            ) from err
    raise RuntimeError(f"Failed to download {url}: exhausted TLS fallbacks")


def main() -> int:
    logging.basicConfig(
        level=logging.INFO,
        format="%(levelname)s: %(message)s",
        stream=sys.stderr,
    )
    parser = argparse.ArgumentParser(
        description=(
            "Create a sysroot based on alpine-minirootfs for cross-compiling "
            "LLVM and bpf-linker for musl targets."
        )
    )
    parser.add_argument("arch", help="Target architecture, e.g. x86_64 or aarch64")
    parser.add_argument(
        "destination",
        help="Directory where the sysroot should be extracted/created",
    )
    parser.add_argument(
        "rust_version",
        nargs="?",
        help="Optional Rust toolchain version to install inside the sysroot",
    )
    args = parser.parse_args()

    arch = args.arch
    arch_upper = arch.upper()
    dest_dir = Path(args.destination).expanduser()
    rust_version = args.rust_version

    base_url = (
        f"https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/{arch}"
    )
    manifest_url = f"{base_url}/latest-releases.yaml"

    LOGGER.info("Downloading Alpine release manifest from %s", manifest_url)
    manifest_bytes = download(manifest_url)
    manifest = manifest_bytes.decode("utf-8")
    pattern = re.compile(
        MINIROOTFS_PATTERN.format(version=r"(\d+\.\d+\.\d+)", arch=re.escape(arch))
    )
    releases: Dict[str, str] = {}
    for match in pattern.finditer(manifest):
        version = match.group(1)
        releases[version] = match.group(0)
    if not releases:
        raise RuntimeError(
            "Could not find any Alpine minirootfs archives for architecture "
            f"{arch} in the release manifest."
        )

    def version_key(version: str) -> Tuple[int, ...]:
        return tuple(int(part) for part in version.split("."))

    version = max(releases.keys(), key=version_key)
    archive_name = releases[version]
    LOGGER.info("Selected Alpine minirootfs release %s (%s)", version, archive_name)

    archive_url = f"{base_url}/{archive_name}"
    LOGGER.info("Downloading Alpine minirootfs archive from %s", archive_url)
    with tempfile.NamedTemporaryFile(delete=False) as tmp:
        tarball = Path(tmp.name)
    try:
        download(archive_url, copy_to=tarball)
        LOGGER.info("Extracting sysroot into %s", dest_dir)
        dest_dir.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [
                "tar",
                "-xpzf",
                str(tarball),
                "--xattrs-include=*.*",
                "--numeric-owner",
                "-C",
                str(dest_dir),
            ],
            check=True,
            stdout=sys.stderr,
            stderr=sys.stderr,
        )
    finally:
        tarball.unlink(missing_ok=True)

    env = os.environ.copy()
    sysroot_value = str(dest_dir.resolve())
    env[f"BPF_LINKER_SYSROOT_{arch_upper}_LINUX_MUSL"] = sysroot_value
    LOGGER.info("Exported BPF_LINKER_SYSROOT_%s_LINUX_MUSL=%s", arch_upper, sysroot_value)
    wrapper_dir = Path(__file__).resolve().parent
    runner = wrapper_dir / f"{arch}-linux-musl-run"
    if not runner.exists():
        raise RuntimeError(f"Runner script not found: {runner}")

    base_packages = [
        "clang",
        "lld",
        "llvm-test-utils",
        "musl-dev",
        "zlib-dev",
        "zlib-static",
        "zstd-dev",
        "zstd-static",
    ]
    script_lines = [
        "set -eu",
        "apk update",
        "apk add \\\n  " + " \\\n  ".join(base_packages),
    ]
    if rust_version:
        script_lines.extend(
            [
                "apk add rustup",
                f"rustup-init -y --default-toolchain {rust_version} --component rust-src",
                '. "$HOME/.cargo/env"',
                "cargo install btfdump",
            ]
        )
    script_content = "\n".join(script_lines) + "\n"
    LOGGER.info("Installing dependencies inside the sysroot (this may take a while)")
    subprocess.run(
        [str(runner), "/bin/sh", "-l"],
        input=script_content,
        text=True,
        check=True,
        env=env,
        stdout=sys.stderr,
        stderr=sys.stderr,
    )
    wrapper_dir = Path(__file__).resolve().parent.resolve()
    sysroot_value = str(dest_dir.resolve())
    print(f"BPF_LINKER_SYSROOT_{arch_upper}_LINUX_MUSL={sysroot_value}")
    print(
        f"CARGO_TARGET_{arch_upper}_UNKNOWN_LINUX_MUSL_LINKER="
        f"{wrapper_dir}/{arch}-linux-musl-clang"
    )
    print(
        f"CARGO_TARGET_{arch_upper}_UNKNOWN_LINUX_MUSL_RUNNER="
        f"{wrapper_dir}/{arch}-linux-musl-run"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # pragma: no cover - script level error reporting
        LOGGER.error("%s", exc)
        raise SystemExit(1) from exc
