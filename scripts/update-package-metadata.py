#!/usr/bin/env python3
"""Update Homebrew and Scoop metadata from a release SHA256SUMS file."""

from __future__ import annotations

import json
import pathlib
import sys

REPOSITORY = "4fuu/cairn"
DESCRIPTION = "Local, read-only index of coding-agent sessions"
MACOS_TARGET = "aarch64-apple-darwin"
WINDOWS_TARGET = "x86_64-pc-windows-msvc"


def fail(message: str) -> None:
    print(f"update-package-metadata: {message}", file=sys.stderr)
    raise SystemExit(1)


def read_checksums(path: pathlib.Path) -> dict[str, str]:
    checksums: dict[str, str] = {}
    for line in path.read_text().splitlines():
        fields = line.split()
        if len(fields) != 2:
            continue
        digest, name = fields
        checksums[name.lstrip("*")] = digest
    return checksums


def main() -> None:
    if len(sys.argv) != 3:
        fail("usage: update-package-metadata.py VERSION SHA256SUMS")

    version = sys.argv[1]
    checksums = read_checksums(pathlib.Path(sys.argv[2]))
    macos_asset = f"cairn-{version}-{MACOS_TARGET}.tar.gz"
    windows_asset = f"cairn-{version}-{WINDOWS_TARGET}.zip"
    for asset in (macos_asset, windows_asset):
        if asset not in checksums:
            fail(f"missing checksum for {asset}")

    base = f"https://github.com/{REPOSITORY}/releases/download/v{version}"
    root = pathlib.Path(__file__).resolve().parents[1]

    formula = root / "Formula" / "cairn.rb"
    formula.parent.mkdir(parents=True, exist_ok=True)
    formula.write_text(
        f"""class Cairn < Formula
  desc "{DESCRIPTION}"
  homepage "https://github.com/{REPOSITORY}"
  version "{version}"
  license "MIT"
  depends_on arch: :arm64

  on_macos do
    url "{base}/{macos_asset}"
    sha256 "{checksums[macos_asset]}"
  end

  def install
    bin.install "cairn"
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/cairn --version")
  end
end
"""
    )

    manifest = {
        "version": version,
        "description": DESCRIPTION,
        "homepage": f"https://github.com/{REPOSITORY}",
        "license": "MIT",
        "architecture": {
            "64bit": {
                "url": f"{base}/{windows_asset}",
                "hash": checksums[windows_asset],
            }
        },
        "bin": "cairn.exe",
        "checkver": {"github": f"https://github.com/{REPOSITORY}"},
        "autoupdate": {
            "architecture": {
                "64bit": {
                    "url": f"https://github.com/{REPOSITORY}/releases/download/v$version/cairn-$version-{WINDOWS_TARGET}.zip"
                }
            }
        },
    }
    bucket = root / "bucket" / "cairn.json"
    bucket.parent.mkdir(parents=True, exist_ok=True)
    bucket.write_text(json.dumps(manifest, indent=4) + "\n")


if __name__ == "__main__":
    main()
