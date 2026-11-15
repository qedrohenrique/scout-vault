"""Gera um cofre sintético para testar o `scout-vault-core` contra uma implementação
independente (biblioteca `cryptography` do Python), sem servidor nem conta real.

Uso:  python tools/gen_fixtures.py
Saída: crates/scout-vault-core/tests/fixtures/{sync.json,vectors.json}

Os IVs e chaves são aleatórios a cada execução; os textos claros e as
expectativas do teste Rust (crates/scout-vault-core/tests/fixtures.rs) não mudam.
"""

import base64
import hashlib
import hmac
import json
import os
from pathlib import Path

from cryptography.hazmat.primitives import hashes, padding, serialization
from cryptography.hazmat.primitives.asymmetric import padding as asym_padding
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from cryptography.hazmat.primitives.kdf.argon2 import Argon2id
from cryptography.hazmat.primitives.kdf.hkdf import HKDFExpand

OUT = Path(__file__).resolve().parent.parent / "crates" / "scout-vault-core" / "tests" / "fixtures"

EMAIL = "  Pessoa.Teste@Example.COM "  # o cliente deve aparar e pôr em minúsculas
PASSWORD = "cavalo correto bateria grampo ção 🔐"
PBKDF2_ITERATIONS = 5000  # baixo de propósito: o teste roda em debug
ARGON = dict(iterations=3, memory_mib=16, parallelism=2)


def b64(data: bytes) -> str:
    return base64.b64encode(data).decode()


def master_key_pbkdf2(password: str, email: str, iterations: int) -> bytes:
    salt = email.strip().lower().encode()
    return hashlib.pbkdf2_hmac("sha256", password.encode(), salt, iterations, 32)


def master_key_argon2(password: str, email: str) -> bytes:
    salt = hashlib.sha256(email.strip().lower().encode()).digest()
    kdf = Argon2id(
        salt=salt,
        length=32,
        iterations=ARGON["iterations"],
        lanes=ARGON["parallelism"],
        memory_cost=ARGON["memory_mib"] * 1024,
    )
    return kdf.derive(password.encode())


def server_hash(master_key: bytes, password: str) -> str:
    return b64(hashlib.pbkdf2_hmac("sha256", master_key, password.encode(), 1, 32))


def stretch(master_key: bytes) -> bytes:
    enc = HKDFExpand(algorithm=hashes.SHA256(), length=32, info=b"enc").derive(master_key)
    mac = HKDFExpand(algorithm=hashes.SHA256(), length=32, info=b"mac").derive(master_key)
    return enc + mac


def encrypt(key64: bytes, plaintext: bytes | str) -> str:
    if isinstance(plaintext, str):
        plaintext = plaintext.encode()
    enc_key, mac_key = key64[:32], key64[32:]
    iv = os.urandom(16)
    padder = padding.PKCS7(128).padder()
    padded = padder.update(plaintext) + padder.finalize()
    encryptor = Cipher(algorithms.AES(enc_key), modes.CBC(iv)).encryptor()
    ct = encryptor.update(padded) + encryptor.finalize()
    mac = hmac.new(mac_key, iv + ct, hashlib.sha256).digest()
    return f"2.{b64(iv)}|{b64(ct)}|{b64(mac)}"


def tamper(enc_string: str) -> str:
    head, rest = enc_string.split(".", 1)
    iv, ct, mac = rest.split("|")
    raw = bytearray(base64.b64decode(ct))
    raw[0] ^= 1
    return f"{head}.{iv}|{b64(bytes(raw))}|{mac}"


def main() -> None:
    mk = master_key_pbkdf2(PASSWORD, EMAIL, PBKDF2_ITERATIONS)
    mk_argon = master_key_argon2(PASSWORD, EMAIL)
    stretched = stretch(mk)

    user_key = os.urandom(64)
    org_key = os.urandom(64)
    item_key = os.urandom(64)

    rsa_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    pkcs8 = rsa_key.private_bytes(
        serialization.Encoding.DER,
        serialization.PrivateFormat.PKCS8,
        serialization.NoEncryption(),
    )
    oaep_sha1 = asym_padding.OAEP(
        mgf=asym_padding.MGF1(algorithm=hashes.SHA1()), algorithm=hashes.SHA1(), label=None
    )
    org_key_enc = "4." + b64(rsa_key.public_key().encrypt(org_key, oaep_sha1))

    u = lambda s: encrypt(user_key, s)  # noqa: E731
    o = lambda s: encrypt(org_key, s)  # noqa: E731
    i = lambda s: encrypt(item_key, s)  # noqa: E731

    sync = {
        "object": "sync",
        "profile": {
            "id": "user-1",
            "email": EMAIL.strip().lower(),
            "key": encrypt(stretched, user_key),
            "privateKey": u(pkcs8),
            "organizations": [
                {"id": "org-1", "name": "Empresa", "key": org_key_enc},
                {"id": "org-quebrada", "name": "Quebrada", "key": "4." + b64(b"lixo")},
            ],
        },
        "folders": [{"id": "folder-1", "name": u("Bancos")}],
        "collections": [
            {"id": "col-fin", "organizationId": "org-1", "name": o("Financeiro"), "readOnly": False},
            {"id": "col-ro", "organizationId": "org-1", "name": o("Só leitura"), "readOnly": True},
        ],
        "ciphers": [
            # Em PascalCase, como o servidor oficial antigo responde.
            {
                "Id": "login-1",
                "Type": 1,
                "FolderId": "folder-1",
                "OrganizationId": None,
                "Favorite": True,
                "Reprompt": 1,
                "Name": u("Meu Banco"),
                "Notes": u("nota com\nduas linhas"),
                "Login": {
                    "Username": u("pessoa"),
                    "Password": u("s3nh@-ção"),
                    "Totp": u("otpauth://totp/Banco?secret=JBSWY3DPEHPK3PXP"),
                    "Uris": [{"Uri": u("https://banco.example.com"), "Match": None}],
                },
                "Fields": [{"Type": 1, "Name": u("PIN"), "Value": u("1234")}],
                "DeletedDate": None,
            },
            {
                "id": "login-item-key",
                "type": 1,
                "name": i("Com chave própria"),
                "key": u(item_key),
                "login": {"username": None, "password": i("item-key-pw"), "uris": None},
                "fields": None,
            },
            {
                "id": "card-org",
                "type": 3,
                "organizationId": "org-1",
                "collectionIds": ["col-fin"],
                "name": o("Cartão da empresa"),
                "card": {
                    "cardholderName": o("Ana Souza"),
                    "brand": o("Visa"),
                    "number": o("4111111111111111"),
                    "expMonth": o("12"),
                    "expYear": o("2030"),
                    "code": o("123"),
                },
            },
            {
                "id": "note-deleted",
                "type": 2,
                "name": u("Lixo"),
                "notes": u("apagado"),
                "secureNote": {"type": 0},
                "deletedDate": "2026-01-01T00:00:00Z",
            },
            {
                "id": "identity-1",
                "type": 4,
                "name": u("Documento"),
                "identity": {"firstName": u("Ana"), "email": u("ana@example.com"), "SSN": u("123-45-6789")},
            },
            {
                "id": "ssh-1",
                "type": 5,
                "name": u("Servidor"),
                "sshKey": {
                    "privateKey": u("-----BEGIN OPENSSH PRIVATE KEY-----\nfake\n-----END OPENSSH PRIVATE KEY-----"),
                    "publicKey": u("ssh-ed25519 AAAAfake"),
                    "keyFingerprint": u("SHA256:fake"),
                },
            },
            {"id": "corrupted", "type": 1, "name": tamper(u("adulterado")), "login": {}},
            {"id": "orphan", "type": 2, "organizationId": "org-quebrada", "name": o("sem chave")},
            {"id": "future-type", "type": 99, "name": u("Tipo novo")},
        ],
    }

    vectors = {
        "email": EMAIL,
        "password": PASSWORD,
        "pbkdf2": {"iterations": PBKDF2_ITERATIONS, "server_hash": server_hash(mk, PASSWORD)},
        "argon2id": {**ARGON, "server_hash": server_hash(mk_argon, PASSWORD)},
    }

    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "sync.json").write_text(json.dumps(sync, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    (OUT / "vectors.json").write_text(json.dumps(vectors, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"fixtures escritas em {OUT}")


if __name__ == "__main__":
    main()
