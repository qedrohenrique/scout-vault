"""Builds the release zip from the already compiled binary.

Usage: python tools/package_release.py 0.3.0 [--exe target/release/scout-vault.exe] [--out dist]

Creates `dist/scout-vault-<version>-windows-x64.zip` with `scout-vault.exe`, the
`extension/` folder (without the test page), `LICENSE`, `README.md` and
`CHANGELOG.md`, and the `.sha256` next to it (same format as `sha256sum`).
"""

from __future__ import annotations

import argparse
import hashlib
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def files(exe: Path) -> list[tuple[Path, str]]:
    out = [(exe, "scout-vault.exe")]
    ext = ROOT / "extension"
    for path in sorted(ext.rglob("*")):
        rel = path.relative_to(ext)
        if path.is_file() and rel.parts[0] != "test":
            out.append((path, f"extension/{rel.as_posix()}"))
    for name in ["LICENSE", "README.md", "CHANGELOG.md"]:
        out.append((ROOT / name, name))
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("version")
    parser.add_argument("--exe", type=Path, default=ROOT / "target" / "release" / "scout-vault.exe")
    parser.add_argument("--out", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    version = args.version.removeprefix("v")
    if not args.exe.is_file():
        sys.exit(f"Could not find the executable at {args.exe}. Run `cargo build --release` first.")
    args.out.mkdir(parents=True, exist_ok=True)
    zip_path = args.out / f"scout-vault-{version}-windows-x64.zip"
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for src, arc in files(args.exe):
            z.write(src, arc)
    digest = hashlib.sha256(zip_path.read_bytes()).hexdigest()
    (args.out / f"{zip_path.name}.sha256").write_text(f"{digest} *{zip_path.name}\n", encoding="utf-8")
    print(zip_path)


if __name__ == "__main__":
    main()
