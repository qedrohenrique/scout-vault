# Roadmap — Cofre

Cliente desktop nativo (Rust + GPUI) para servidores compatíveis com Bitwarden
(Bitwarden oficial e Vaultwarden). Objetivo: um app leve e rápido que substitua
o desktop Electron no uso diário — buscar, copiar e editar itens do cofre.

**Princípios**
- Nada decifrado vai para o disco. Segredos vivem em memória com `zeroize`.
- `cofre-core` não conhece UI: todo o protocolo e a criptografia são testáveis sozinhos.
- Desenvolvimento sempre contra um **Vaultwarden local** (`dev/docker-compose.yml`), nunca contra o cofre real.
- Referências de implementação: [`rbw`](https://github.com/doy/rbw) (MIT) e a documentação de segurança do Bitwarden.
  Não copiar código do SDK oficial (licença restritiva).

---

## Fase 0 — Fundação ✅ (esqueleto pronto)
- [x] Workspace `cofre-core` + `cofre-app`
- [x] Janela GPUI abrindo (gpui-kit)
- [x] Parse de `EncString` (tipos 2 e 4) e parâmetros de KDF, com testes
- [x] Vaultwarden local via Docker
- [x] CI (Windows): fmt, clippy, test
- [x] Instalar toolchain e validar `cargo run` localmente

## Fase 1 — Núcleo: login e leitura do cofre (`cofre-core`) 🟡 (código pronto, falta validar em servidor real)
Meta: um teste de integração que loga no Vaultwarden local e lista os nomes dos itens decifrados.

- [x] **Prelogin** — `POST /identity/accounts/prelogin` → `Kdf` (`api::Client::prelogin`)
- [x] **Master key** — PBKDF2-SHA256 (salt = e-mail aparado e minúsculo) ou Argon2id (salt = SHA-256 do e-mail) (`crypto::MasterKey::derive`)
- [x] **Hash de autenticação** — PBKDF2-SHA256(master key, salt = senha, 1 iteração); a senha nunca sai da máquina
- [x] **Login** — `POST /identity/connect/token` com `grant_type=password`, dados do dispositivo e `Auth-Email`
  - [x] 2FA: detecta `TwoFactorProviders` (números ou strings) e reenvia com `twoFactorProvider/Token/Remember`
  - [x] Pedido de código de 2FA por e-mail (`send_email_login_code`)
  - [x] Verificação de novo dispositivo: detectada e reenviada com `newDeviceOtp` — ⚠️ nome do campo **não confirmado** em servidor real
  - [ ] Login por API key pessoal (`client_credentials`) — opcional
- [x] **Stretch** — HKDF-Expand-SHA256 → `enc` + `mac`
- [x] **Decifrar/cifrar `EncString` tipo 2** — HMAC verificado em tempo constante antes do AES-256-CBC
- [x] **User key**, **chave privada RSA** (PKCS#8) e **chaves de organização** (RSA-OAEP-SHA1)
- [x] **Sync** — `GET /api/sync`; JSON normalizado para camelCase (aceita servidor antigo em PascalCase)
- [x] **Chave própria por item** (`cipher.key`)
- [x] **Refresh token**
- [x] **Modelo decifrado** — login, nota, cartão, identidade, chave SSH, campos customizados, lixeira, busca
- [x] Falhas isoladas: item ou organização corrompidos não derrubam o resto do cofre
- [x] Segredos em `Secret`/`Zeroizing`, `Debug` sem material de chave
- [ ] **Validar contra o Vaultwarden local** (`tests/live_server.rs`) — precisa do Docker
- [ ] Validar contra o Bitwarden oficial com uma conta de teste (2FA, novo dispositivo)

**Testes:** `tools/gen_fixtures.py` gera, com a biblioteca `cryptography` do Python (implementação
independente), um cofre sintético completo — KDFs, user key, RSA, organização, chave de item,
PascalCase, item adulterado. `tests/fixtures.rs` precisa abrir tudo e bater cada valor.

## Fase 2 — MVP de leitura (`cofre-app`)
Meta: usar no dia a dia para **consultar** senhas.

- Tela de **configuração do servidor** (URL; padrão `https://vault.bitwarden.com`, ou self-hosted).
- Tela de **login** (e-mail, senha, 2FA) e de **desbloqueio** (só senha mestra, reaproveitando o refresh token).
- **Lista do cofre** virtualizada com busca instantânea (nome, usuário, URI), filtros por tipo/pasta/organização.
- **Detalhe do item** com campos mascarados e botão de revelar.
- **Copiar para a área de transferência** com limpeza automática após N segundos (`arboard`).
- **Bloqueio automático** por inatividade e ao minimizar/suspender.
- Async: rede/cripto em background (`tokio` ou o executor do GPUI), UI nunca trava — Argon2 é lento de propósito.

## Fase 3 — Persistência segura e conveniência
- **Refresh token** no cofre do SO via `keyring` (Windows Credential Manager).
- **Cache local cifrado** do sync (os dados já vêm cifrados do servidor — guardar o JSON cru, decifrar só na memória) → abrir offline.
- **Sync incremental**/periódico e via notificações (`/notifications/hub`, SignalR) — opcional.
- **Gerador de TOTP** para itens com chave de autenticador (`totp-rs`), com contador visual.
- **Gerador de senhas e passphrases**.
- **Atalho global** (ex.: `Ctrl+Shift+L`) abrindo uma janela de busca rápida estilo launcher.
- Desbloqueio com **Windows Hello** (pesquisa: chave protegida por TPM/DPAPI).

## Fase 4 — Escrita
- Cifrar: AES-256-CBC + HMAC com IV aleatório (`rand`/`getrandom`), gerar `EncString`.
- Criar / editar / mover para lixeira / restaurar itens (`POST/PUT /api/ciphers`, `PUT /api/ciphers/{id}/delete`).
- Pastas: criar, renomear, apagar.
- Histórico de senhas do item.
- Testes de ida e volta: cifrar no Cofre → ler na CLI oficial/web vault (e vice-versa).

## Fase 5 — Recursos avançados
- Organizações e coleções (escrita), compartilhamento de item.
- Anexos (download/upload, cifrados com chave própria).
- Bitwarden **Send**.
- Relatórios locais: senhas reutilizadas/fracas; senhas vazadas via k-anonymity da API do HIBP.
- Importar/exportar (JSON do Bitwarden, cifrado).
- Agente SSH (o Bitwarden guarda chaves SSH) — opcional e ambicioso.

## Fase 6 — Distribuição
- Instalador Windows (MSI via `cargo-wix` ou `cargo-dist`), ícone, metadados.
- Builds para Linux e macOS na CI.
- Atualização automática.
- Assinatura de código (opcional).

---

## Marcos
| Marco | Conteúdo | Demonstrável |
|---|---|---|
| M1 | Fase 1 | Teste lista itens decifrados do Vaultwarden local |
| M2 | Fase 2 | App substitui o desktop oficial para **consultar** |
| M3 | Fase 3 | Uso diário confortável (offline, TOTP, atalho global) |
| M4 | Fase 4 | Substitui o desktop oficial por completo no uso pessoal |

## Riscos e decisões em aberto
- **API não documentada formalmente** — o servidor pode mudar requisitos de login; acompanhar `rbw` e o changelog do Vaultwarden.
- **Revisão de segurança** — antes de usar com o cofre real, revisar a cripto contra vetores de teste e, idealmente, pedir revisão externa.
- **Marca** — não usar nome/logo "Bitwarden"; descrever como "compatível com Bitwarden", projeto não oficial.
- **Licença** do projeto — a definir (MIT/Apache-2.0 é o padrão em Rust; gpui-kit é Apache-2.0).
