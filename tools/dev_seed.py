"""Cria uma conta de TESTE no Vaultwarden local (dev/docker-compose.yml) e
popula alguns itens, sem abrir navegador. Grava as credenciais em
dev/test-account.env (ignorado pelo git) para o teste ao vivo.

Uso:  python tools/dev_seed.py [--url http://localhost:8000] [--totp]

Com --totp, a conta ganha 2FA por autenticador e as credenciais (com o segredo
TOTP) vão para dev/test-account-2fa.env.

Só aceita servidores em localhost — nunca aponte para um servidor real.
"""

import argparse
import base64
import hashlib
import hmac
import struct
import time
import json
import secrets
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path
from urllib.parse import urlparse

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import rsa

from gen_fixtures import b64, encrypt, master_key_pbkdf2, server_hash, stretch

ROOT = Path(__file__).resolve().parent.parent
ENV_FILE = ROOT / "dev" / "test-account.env"
ENV_FILE_2FA = ROOT / "dev" / "test-account-2fa.env"
ITERATIONS = 600_000


def request(url: str, *, json_body=None, form=None, token=None, headers=None):
    data, hdrs = None, dict(headers or {})
    if json_body is not None:
        data = json.dumps(json_body).encode()
        hdrs["Content-Type"] = "application/json"
    elif form is not None:
        data = urllib.parse.urlencode(form).encode()
        hdrs["Content-Type"] = "application/x-www-form-urlencoded"
    if token:
        hdrs["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, data=data, headers=hdrs, method="POST" if data else "GET")
    try:
        with urllib.request.urlopen(req, timeout=30) as res:
            body = res.read()
            return json.loads(body) if body else None
    except urllib.error.HTTPError as e:
        sys.exit(f"{url} -> {e.code}: {e.read().decode(errors='replace')}")


def totp(secret_b32: str, at: float | None = None) -> str:
    key = base64.b32decode(secret_b32 + "=" * (-len(secret_b32) % 8))
    counter = int((time.time() if at is None else at) // 30)
    digest = hmac.new(key, struct.pack(">Q", counter), hashlib.sha1).digest()
    offset = digest[-1] & 0x0F
    code = (struct.unpack(">I", digest[offset : offset + 4])[0] & 0x7FFFFFFF) % 1_000_000
    return f"{code:06d}"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://localhost:8000")
    ap.add_argument("--totp", action="store_true", help="ativa 2FA por autenticador")
    args = ap.parse_args()
    base = args.url.rstrip("/")
    if urlparse(base).hostname not in {"localhost", "127.0.0.1", "::1"}:
        sys.exit("recusado: dev_seed só roda contra servidor local")

    email = f"teste-{secrets.token_hex(3)}@cofre.test"
    password = secrets.token_urlsafe(18)

    mk = master_key_pbkdf2(password, email, ITERATIONS)
    user_key = secrets.token_bytes(64)
    rsa_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    pkcs8 = rsa_key.private_bytes(
        serialization.Encoding.DER, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()
    )
    spki = rsa_key.public_key().public_bytes(
        serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo
    )

    # Fluxo atual: sem SMTP configurado, o servidor devolve direto o token de verificação.
    verification_token = request(
        f"{base}/identity/accounts/register/send-verification-email",
        json_body={"email": email, "name": "Conta de teste do Scout Vault"},
        headers={"Accept": "application/json"},
    )
    if not isinstance(verification_token, str):
        sys.exit("o servidor não devolveu token de verificação (SMTP ligado?)")

    request(
        f"{base}/identity/accounts/register/finish",
        json_body={
            "email": email,
            "emailVerificationToken": verification_token,
            "name": "Conta de teste do Scout Vault",
            "masterPasswordHash": server_hash(mk, password),
            "masterPasswordHint": None,
            "key": encrypt(stretch(mk), user_key),
            "keys": {"publicKey": b64(spki), "encryptedPrivateKey": encrypt(user_key, pkcs8)},
            "kdf": 0,
            "kdfIterations": ITERATIONS,
        },
    )

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
            "deviceName": "scout-vault-dev-seed",
        },
        headers={"Auth-Email": base64.urlsafe_b64encode(email.encode()).decode().rstrip("=")},
    )
    token = tokens["access_token"]
    payload = token.split(".")[1]
    user_id = json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))["sub"]
    u = lambda s: encrypt(user_key, s)  # noqa: E731

    folder = request(f"{base}/api/folders", json_body={"name": u("Bancos")}, token=token)

    items = [
        {
            "type": 1,
            "folderId": folder["id"],
            "favorite": True,
            "name": u("Meu Banco"),
            "notes": u("criado pelo dev_seed"),
            "login": {
                "username": u("pessoa"),
                "password": u("s3nh@-ção"),
                "totp": u("JBSWY3DPEHPK3PXP"),
                "uris": [{"uri": u("https://banco.example.com"), "match": None}],
            },
            "fields": [{"type": 1, "name": u("PIN"), "value": u("1234")}],
        },
        {
            "type": 3,
            "name": u("Cartão pessoal"),
            "card": {
                "cardholderName": u("Pessoa Teste"),
                "brand": u("Visa"),
                "number": u("4111111111111111"),
                "expMonth": u("12"),
                "expYear": u("2030"),
                "code": u("123"),
            },
        },
        {"type": 2, "name": u("Nota segura"), "notes": u("conteúdo secreto"), "secureNote": {"type": 0}},
    ]
    item_key = secrets.token_bytes(64)
    items.append(
        {
            "type": 1,
            "key": u(item_key),
            "name": encrypt(item_key, "Com chave própria"),
            "login": {"username": encrypt(item_key, "outra"), "password": encrypt(item_key, "item-key-pw")},
        }
    )
    for item in items:
        item["encryptedFor"] = user_id  # exigido desde web-v2025.6.0
        request(f"{base}/api/ciphers", json_body=item, token=token)

    if args.totp:
        # O servidor sugere a chave e devolve um token de verificação que a ativação exige.
        setup = request(
            f"{base}/api/two-factor/get-authenticator",
            json_body={"masterPasswordHash": server_hash(mk, password)},
            token=token,
        )
        secret = setup["authenticator"]["key"]
        request(
            f"{base}/api/two-factor/authenticator",
            json_body={
                "key": secret,
                "token": totp(secret),
                "userVerificationToken": setup["userVerificationToken"],
            },
            token=token,
        )
        ENV_FILE_2FA.write_text(
            f"SCOUT_VAULT_TEST_2FA_URL={base}\nSCOUT_VAULT_TEST_2FA_EMAIL={email}\n"
            f"SCOUT_VAULT_TEST_2FA_PASSWORD={password}\nSCOUT_VAULT_TEST_2FA_TOTP_SECRET={secret}\n",
            encoding="utf-8",
        )
        print(f"conta {email} criada com 2FA por autenticador; credenciais em {ENV_FILE_2FA}")
        return

    ENV_FILE.write_text(
        f"SCOUT_VAULT_TEST_URL={base}\nSCOUT_VAULT_TEST_EMAIL={email}\nSCOUT_VAULT_TEST_PASSWORD={password}\n",
        encoding="utf-8",
    )
    print(f"conta {email} criada com 1 pasta e {len(items)} itens; credenciais em {ENV_FILE}")


if __name__ == "__main__":
    main()
