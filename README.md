# Cofre

Cliente desktop nativo, em **Rust + GPUI**, para servidores compatíveis com Bitwarden
(Bitwarden oficial e Vaultwarden).

> Projeto não oficial, sem afiliação com a Bitwarden Inc. **Em desenvolvimento — não use com seu cofre real ainda.**

## Estrutura
```
crates/
  cofre-core/   protocolo, criptografia e modelo do cofre (sem UI, testável)
  cofre-app/    interface GPUI (binário `cofre`)
dev/            Vaultwarden local para desenvolvimento
docs/ROADMAP.md plano de implementação por fases
```

## Pré-requisitos (Windows)
- Rust estável: `winget install Rustlang.Rustup`
- Visual Studio Build Tools com a carga "Desenvolvimento para desktop com C++" (linker MSVC + Windows SDK)
- Docker Desktop (só para o servidor de desenvolvimento)

## Rodando
```bash
cargo test -p cofre-core   # testes do núcleo
cargo run                  # abre o app
```

## Servidor de desenvolvimento
Vaultwarden local em `http://localhost:8000`. Funciona com Docker Desktop ou com o Docker
instalado dentro do WSL (sem Docker Desktop):
```bash
wsl -d Ubuntu -u root -- bash -c "service docker start; cd /mnt/d/Repositories/cofre/dev && docker compose up -d"
```
O WSL desliga quando fica ocioso; mantenha um terminal do WSL aberto enquanto testa.

Crie uma conta de teste já populada (sem navegador; credenciais vão para `dev/test-account.env`, ignorado pelo git):
```bash
python tools/dev_seed.py
```

Teste ao vivo (login, sync e decifragem):
```bash
set -a; . dev/test-account.env; set +a
cargo test -p cofre-core --test live_server -- --ignored --nocapture
```

## Roadmap
Veja [docs/ROADMAP.md](docs/ROADMAP.md).
