"""Signs the release files with the project's Ed25519 key.

Usage: RELEASE_SIGNING_KEY=<base64 seed> python tools/sign_release.py dist

Writes `<file>.sig` (base64 of the 64-byte signature) next to every `.exe` and `.zip`
in the folder. The app checks the installer's signature with the public key built
into it (`scout_vault_core::update::RELEASE_PUBLIC_KEY`) before running an update.

The key comes from the `RELEASE_SIGNING_KEY` secret of the release workflow; without
it the script fails, so an unsigned release is never published by accident.
"""

from __future__ import annotations

import base64
import os
import sys
from pathlib import Path

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization

# Same value as `RELEASE_PUBLIC_KEY` in crates/scout-vault-core/src/update.rs.
EXPECTED_PUBLIC_KEY = "xN9t1ZichP3gJX7x4NsXGSb1pbFyvUtQT7NNU6XpmcE="


def main() -> int:
    folder = Path(sys.argv[1] if len(sys.argv) > 1 else "dist")
    seed = os.environ.get("RELEASE_SIGNING_KEY", "").strip()
    if not seed:
        print("RELEASE_SIGNING_KEY is not set: refusing to publish unsigned files", file=sys.stderr)
        return 1
    key = Ed25519PrivateKey.from_private_bytes(base64.b64decode(seed))
    public = base64.b64encode(
        key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    ).decode()
    expected = os.environ.get("RELEASE_PUBLIC_KEY", EXPECTED_PUBLIC_KEY)
    if public != expected:
        print("the signing key doesn't match the public key built into the app", file=sys.stderr)
        return 1
    files = sorted(p for p in folder.iterdir() if p.suffix in (".exe", ".zip"))
    if not files:
        print(f"nothing to sign in {folder}", file=sys.stderr)
        return 1
    for path in files:
        signature = key.sign(path.read_bytes())
        Path(f"{path}.sig").write_text(base64.b64encode(signature).decode(), encoding="ascii")
        print(f"signed {path.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
