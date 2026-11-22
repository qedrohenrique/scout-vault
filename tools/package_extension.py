"""Builds the browser extension packages for the stores.

Usage: python tools/package_extension.py 0.13.0 [--out dist]

- `scout-vault-extension-<version>-chromium.zip`: Chrome Web Store and Edge Add-ons.
  The manifest's `key` is removed (the stores refuse it and give the extension an id
  of their own; add that id to `STORE_EXTENSION_IDS` in `browser.rs` once published).
- `scout-vault-extension-<version>-firefox.zip`: addons.mozilla.org. Same code, with
  the Firefox id (`FIREFOX_EXTENSION_ID` in `browser.rs`) and a background script
  instead of a service worker.

The `test/` pages stay out.
"""

from __future__ import annotations

import argparse
import json
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXTENSION = ROOT / "extension"
FIREFOX_ID = "scout-vault@qedrohenrique.github.io"


def files() -> list[Path]:
    return sorted(
        p
        for p in EXTENSION.rglob("*")
        if p.is_file() and "test" not in p.relative_to(EXTENSION).parts and p.name != "manifest.json"
    )


def chromium_manifest(manifest: dict) -> dict:
    out = dict(manifest)
    out.pop("key", None)
    return out


def firefox_manifest(manifest: dict) -> dict:
    out = chromium_manifest(manifest)
    out["background"] = {"scripts": ["background.js"]}
    out["browser_specific_settings"] = {
        "gecko": {
            "id": FIREFOX_ID,
            "strict_min_version": "128.0",
            # The extension sends nothing anywhere but the Scout Vault app on this computer.
            "data_collection_permissions": {"required": ["none"]},
        }
    }
    return out


def build(path: Path, manifest: dict) -> None:
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("manifest.json", json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
        for f in files():
            z.write(f, f.relative_to(EXTENSION).as_posix())


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("version")
    parser.add_argument("--out", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    version = args.version.lstrip("v")
    manifest = json.loads((EXTENSION / "manifest.json").read_text(encoding="utf-8"))
    if manifest.get("version") != version:
        print(f"extension/manifest.json is {manifest.get('version')}, not {version}", file=sys.stderr)
        return 1
    args.out.mkdir(parents=True, exist_ok=True)
    for kind, transform in (("chromium", chromium_manifest), ("firefox", firefox_manifest)):
        path = args.out / f"scout-vault-extension-{version}-{kind}.zip"
        build(path, transform(manifest))
        print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
