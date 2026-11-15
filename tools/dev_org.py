"""Cria uma organização de teste ("Empresa de teste", coleções Financeiro e TI)
na conta de `dev/test-account.env`, no Vaultwarden LOCAL.

Faz a parte de cliente como os apps oficiais: gera a chave da organização,
cifra-a com a chave pública RSA do usuário e cifra os nomes das coleções com
ela. Se a organização já existir, não faz nada.

    python tools/dev_org.py
"""

import base64
import json
import os
import sys
import uuid
from pathlib import Path
from urllib.parse import urlparse

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding as asym_padding
from cryptography.hazmat.primitives.asymmetric import rsa

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dev_seed import request  # noqa: E402
from gen_fixtures import b64, encrypt, master_key_pbkdf2, server_hash  # noqa: E402

ORG_NAME = "Empresa de teste"
ENV = Path(__file__).resolve().parent.parent / "dev" / "test-account.env"


def load_env() -> dict:
    out = {}
    for line in ENV.read_text(encoding="utf-8").splitlines():
        if "=" in line and not line.lstrip().startswith("#"):
            k, v = line.split("=", 1)
            out[k.strip()] = v.strip().strip('"')
    return out


def main() -> None:
    env = load_env()
    base = env["SCOUT_VAULT_TEST_URL"].rstrip("/")
    if urlparse(base).hostname not in {"localhost", "127.0.0.1", "::1"}:
        sys.exit("recusado: dev_org só roda contra servidor local")
    email, password = env["SCOUT_VAULT_TEST_EMAIL"], env["SCOUT_VAULT_TEST_PASSWORD"]

    kdf = request(f"{base}/identity/accounts/prelogin", json_body={"email": email})
    iterations = kdf.get("kdfIterations") or kdf.get("KdfIterations")
    mk = master_key_pbkdf2(password, email, iterations)
    tokens = request(
        f"{base}/identity/connect/token",
        form={
            "grant_type": "password",
            "username": email,
            "password": server_hash(mk, password),
            "scope": "api offline_access",
            "client_id": "web",
            "deviceType": 9,
            "deviceIdentifier": str(uuid.uuid4()),
            "deviceName": "scout-vault-dev-org",
        },
        headers={"Auth-Email": base64.urlsafe_b64encode(email.encode()).decode().rstrip("=")},
    )
    token = tokens["access_token"]
    payload = token.split(".")[1]
    user_id = json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))["sub"]

    sync = request(f"{base}/api/sync", token=token)
    profile = sync.get("profile") or sync.get("Profile")
    orgs = profile.get("organizations") or profile.get("Organizations") or []
    if any((o.get("name") or o.get("Name")) == ORG_NAME for o in orgs):
        print(f"já existe: {ORG_NAME}")
        return

    pub = request(f"{base}/api/users/{user_id}/public-key", token=token)
    user_pub = serialization.load_der_public_key(base64.b64decode(pub.get("publicKey") or pub["PublicKey"]))

    org_key = os.urandom(64)
    oaep = asym_padding.OAEP(mgf=asym_padding.MGF1(hashes.SHA1()), algorithm=hashes.SHA1(), label=None)
    org_rsa = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    org = request(
        f"{base}/api/organizations",
        json_body={
            "name": ORG_NAME,
            "billingEmail": email,
            "collectionName": encrypt(org_key, "Financeiro"),
            "key": "4." + b64(user_pub.encrypt(org_key, oaep)),
            "keys": {
                "publicKey": b64(
                    org_rsa.public_key().public_bytes(
                        serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo
                    )
                ),
                "encryptedPrivateKey": encrypt(
                    org_key,
                    org_rsa.private_bytes(
                        serialization.Encoding.DER,
                        serialization.PrivateFormat.PKCS8,
                        serialization.NoEncryption(),
                    ),
                ),
            },
            "planType": 0,
        },
        token=token,
    )
    org_id = org.get("id") or org["Id"]

    # Segunda coleção, com acesso para a própria conta.
    members = request(f"{base}/api/organizations/{org_id}/users", token=token)
    members = members.get("data") or members.get("Data") or []
    me = next(m for m in members if (m.get("email") or m.get("Email", "")).lower() == email.lower())
    request(
        f"{base}/api/organizations/{org_id}/collections",
        json_body={
            "name": encrypt(org_key, "TI"),
            "groups": [],
            "users": [{"id": me.get("id") or me["Id"], "readOnly": False, "hidePasswords": False, "manage": True}],
        },
        token=token,
    )
    print(f"criada: {ORG_NAME} (coleções Financeiro e TI)")


if __name__ == "__main__":
    main()
