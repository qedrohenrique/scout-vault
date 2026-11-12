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
```bash
docker compose -f dev/docker-compose.yml up -d
```
Abra http://localhost:8000, crie uma conta de teste e use `http://localhost:8000` como URL do servidor no app.

## Roadmap
Veja [docs/ROADMAP.md](docs/ROADMAP.md).
