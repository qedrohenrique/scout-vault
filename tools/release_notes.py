"""Release notes from CHANGELOG.md.

Usage: python tools/release_notes.py 0.3.0 [--out notes.md]

Checks that the version matches `Cargo.toml` and that the CHANGELOG has the section
`## [0.3.0] - YYYY-MM-DD`; prints that section with the test-version notice and
the usage instructions. Exits with an error (and explains) if something doesn't
match — so the release never ships with wrong notes.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

HEADER = """> ⚠️ **Test version.** The cryptography passes the official test vectors, but the app has not \
had an external security review yet. Keep the official app or the web vault as a backup. \
Unofficial project, not affiliated with Bitwarden Inc.
"""

FOOTER = """## How to use
1. Download `scout-vault-{version}-windows-x64.zip`, extract it to a folder and open `scout-vault.exe` \
(if an earlier version is running, quit it from the tray first).
2. The executable is not signed: if Windows shows "Windows protected your PC", click \
**More info → Run anyway**.
3. Requires 64-bit Windows 10/11. If you see "VCRUNTIME140.dll was not found", install the \
Microsoft Visual C++ Redistributable (x64).

The browser extension is in the zip's `extension` folder (Settings → Browser explains how to \
load it). Verify the file against the `.sha256` published alongside it.
"""


def cargo_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    section = text.split("[workspace.package]", 1)[1]
    match = re.search(r'^version\s*=\s*"([^"]+)"', section, re.M)
    if not match:
        sys.exit("Cargo.toml: could not find the version in [workspace.package].")
    return match.group(1)


def changelog_section(version: str) -> str:
    text = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    heading = re.compile(rf"^## \[{re.escape(version)}\] - \d{{4}}-\d{{2}}-\d{{2}}\s*$", re.M)
    match = heading.search(text)
    if not match:
        sys.exit(
            f"CHANGELOG.md: missing section '## [{version}] - YYYY-MM-DD'. "
            "Rename '## [Unreleased]' to the version and date before tagging."
        )
    rest = text[match.end():]
    end = re.search(r"^## \[|^\[[^\]]+\]: ", rest, re.M)
    body = (rest[: end.start()] if end else rest).strip()
    if not body:
        sys.exit(f"CHANGELOG.md: section {version} is empty.")
    # "### Added" headings become "## Added" on the release page.
    return re.sub(r"^### ", "## ", body, flags=re.M)


def notes(version: str) -> str:
    cargo = cargo_version()
    if cargo != version:
        sys.exit(f"The tag is {version}, but Cargo.toml is at {cargo}. Fix one of them.")
    # The "test version" notice is for 0.x and pre-releases (`1.1.0-rc.1`), not 1.0 on.
    test = version.startswith("0.") or "-" in version
    parts = [HEADER] if test else []
    return "\n".join([*parts, changelog_section(version), "", FOOTER.format(version=version)])


def main() -> None:
    sys.stderr.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("version", help="version without the 'v' (e.g. 0.3.0); 'v0.3.0' is accepted")
    parser.add_argument("--out", type=Path, help="write to a file instead of printing")
    args = parser.parse_args()
    text = notes(args.version.removeprefix("v"))
    if args.out:
        args.out.write_text(text, encoding="utf-8")
    else:
        sys.stdout.reconfigure(encoding="utf-8")
        print(text)


if __name__ == "__main__":
    main()
