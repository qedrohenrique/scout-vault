"""Monta o zip da release a partir do binário já compilado.

Uso: python tools/package_release.py 0.3.0 [--exe target/release/scout-vault.exe] [--out dist]

Gera `dist/scout-vault-<versão>-windows-x64.zip` com `scout-vault.exe`, a
pasta `extension/` (sem a página de teste), `LICENSE`, `README.md` e
`CHANGELOG.md`, e o `.sha256` ao lado (mesmo formato do `sha256sum`).
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
        sys.exit(f"Não achei o executável em {args.exe}. Rode `cargo build --release` antes.")
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
