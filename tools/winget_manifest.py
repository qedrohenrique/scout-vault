"""Writes the winget manifests for a release (to submit to microsoft/winget-pkgs).

Usage: python tools/winget_manifest.py 0.13.0 [--dist dist] [--out dist/winget]

Reads the installer's SHA-256 from `dist/scout-vault-<version>-setup.exe.sha256` and
writes `<out>/<version>/qedrohenrique.ScoutVault*.yaml`. Submit them with
`wingetcreate submit <folder>` or a pull request to microsoft/winget-pkgs.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ID = "qedrohenrique.ScoutVault"
REPO = "https://github.com/qedrohenrique/scout-vault"
MANIFEST_VERSION = "1.9.0"
SCHEMA = "https://aka.ms/winget-manifest.{kind}.1.9.0.schema.json"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("version")
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    parser.add_argument("--out", type=Path, default=None)
    args = parser.parse_args()
    version = args.version.lstrip("v")
    installer = f"scout-vault-{version}-setup.exe"
    sum_file = args.dist / f"{installer}.sha256"
    if not sum_file.is_file():
        print(f"missing {sum_file}", file=sys.stderr)
        return 1
    sha256 = sum_file.read_text(encoding="ascii").split()[0].upper()
    out = (args.out or args.dist / "winget") / version
    out.mkdir(parents=True, exist_ok=True)

    files = {
        f"{ID}.yaml": f"""# yaml-language-server: $schema={SCHEMA.format(kind="version")}
PackageIdentifier: {ID}
PackageVersion: {version}
DefaultLocale: en-US
ManifestType: version
ManifestVersion: {MANIFEST_VERSION}
""",
        f"{ID}.installer.yaml": f"""# yaml-language-server: $schema={SCHEMA.format(kind="installer")}
PackageIdentifier: {ID}
PackageVersion: {version}
InstallerType: inno
Scope: user
InstallModes:
  - interactive
  - silent
  - silentWithProgress
UpgradeBehavior: install
MinimumOSVersion: 10.0.17763.0
Installers:
  - Architecture: x64
    InstallerUrl: {REPO}/releases/download/v{version}/{installer}
    InstallerSha256: {sha256}
ManifestType: installer
ManifestVersion: {MANIFEST_VERSION}
""",
        f"{ID}.locale.en-US.yaml": f"""# yaml-language-server: $schema={SCHEMA.format(kind="defaultLocale")}
PackageIdentifier: {ID}
PackageVersion: {version}
PackageLocale: en-US
Publisher: Scout Vault contributors
PublisherUrl: {REPO}
PublisherSupportUrl: {REPO}/issues
PrivacyUrl: {REPO}/blob/main/docs/PRIVACY.md
PackageName: Scout Vault
PackageUrl: {REPO}
License: MIT
LicenseUrl: {REPO}/blob/main/LICENSE
ShortDescription: Unofficial Bitwarden-compatible password manager for Windows.
Description: Desktop password manager compatible with Bitwarden and Vaultwarden, written in Rust. Fills logins in apps and the browser, SSH agent, Windows Hello unlock, everything encrypted on the computer.
Moniker: scout-vault
Tags:
  - bitwarden
  - vaultwarden
  - password-manager
  - passwords
  - security
ReleaseNotesUrl: {REPO}/releases/tag/v{version}
ManifestType: defaultLocale
ManifestVersion: {MANIFEST_VERSION}
""",
    }
    for name, text in files.items():
        (out / name).write_text(text, encoding="utf-8", newline="\n")
        print(f"wrote {out / name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
