"""Notas de uma release a partir do CHANGELOG.md.

Uso: python tools/release_notes.py 0.3.0 [--out notas.md]

Confere que a versão bate com o `Cargo.toml` e que o CHANGELOG tem a seção
`## [0.3.0] - AAAA-MM-DD`; imprime essa seção com o aviso de versão de teste e
as instruções de uso. Sai com erro (e explica) se algo não bater — assim a
release não sai com notas erradas.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

HEADER = """> ⚠️ **Versão de teste.** A criptografia passa nos vetores oficiais, mas o app ainda não teve \
revisão de segurança externa. Mantenha o app oficial ou o cofre web como reserva. Projeto não \
oficial, sem afiliação com a Bitwarden Inc.
"""

FOOTER = """## Como usar
1. Baixe `scout-vault-{version}-windows-x64.zip`, extraia numa pasta e abra `scout-vault.exe` \
(se uma versão anterior estiver aberta, saia dela pela bandeja antes).
2. O executável não é assinado: se o Windows mostrar "O Windows protegeu o computador", clique em \
**Mais informações → Executar assim mesmo**.
3. Precisa do Windows 10/11 64 bits. Se aparecer "VCRUNTIME140.dll não encontrado", instale o \
Microsoft Visual C++ Redistributable (x64).

A extensão do navegador fica na pasta `extension` do zip (Configurações → Navegador explica como \
carregar). Confira o arquivo com o `.sha256` publicado junto.
"""


def cargo_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    section = text.split("[workspace.package]", 1)[1]
    match = re.search(r'^version\s*=\s*"([^"]+)"', section, re.M)
    if not match:
        sys.exit("Cargo.toml: não achei a versão em [workspace.package].")
    return match.group(1)


def changelog_section(version: str) -> str:
    text = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    heading = re.compile(rf"^## \[{re.escape(version)}\] - \d{{4}}-\d{{2}}-\d{{2}}\s*$", re.M)
    match = heading.search(text)
    if not match:
        sys.exit(
            f"CHANGELOG.md: falta a seção '## [{version}] - AAAA-MM-DD'. "
            "Renomeie '## [Ainda não lançado]' para a versão e a data antes da tag."
        )
    rest = text[match.end():]
    end = re.search(r"^## \[|^\[[^\]]+\]: ", rest, re.M)
    body = (rest[: end.start()] if end else rest).strip()
    if not body:
        sys.exit(f"CHANGELOG.md: a seção {version} está vazia.")
    # Títulos "### Adicionado" viram "## Adicionado" na página da release.
    return re.sub(r"^### ", "## ", body, flags=re.M)


def notes(version: str) -> str:
    cargo = cargo_version()
    if cargo != version:
        sys.exit(f"A tag é {version}, mas o Cargo.toml está em {cargo}. Ajuste um dos dois.")
    return "\n".join([HEADER, changelog_section(version), "", FOOTER.format(version=version)])


def main() -> None:
    sys.stderr.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("version", help="versão sem o 'v' (ex.: 0.3.0); aceita 'v0.3.0'")
    parser.add_argument("--out", type=Path, help="grava num arquivo em vez de imprimir")
    args = parser.parse_args()
    text = notes(args.version.removeprefix("v"))
    if args.out:
        args.out.write_text(text, encoding="utf-8")
    else:
        sys.stdout.reconfigure(encoding="utf-8")
        print(text)


if __name__ == "__main__":
    main()
