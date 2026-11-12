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

## Fase 1 — Núcleo: login e leitura do cofre (`cofre-core`) ✅ (validado no Vaultwarden local)
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
- [x] **Validar contra o Vaultwarden local** (`tests/live_server.rs` + `tools/dev_seed.py`) — login, refresh, sync e 4 itens decifrados sem erro
- [ ] Validar contra o Bitwarden oficial com uma conta de teste (2FA, novo dispositivo — o Vaultwarden não implementa essa verificação)
- [ ] Novos tipos de item do servidor: 6 = conta bancária, 7 = CNH, 8 = passaporte (hoje aparecem como `Unknown`)

**Testes:** `tools/gen_fixtures.py` gera, com a biblioteca `cryptography` do Python (implementação
independente), um cofre sintético completo — KDFs, user key, RSA, organização, chave de item,
PascalCase, item adulterado. `tests/fixtures.rs` precisa abrir tudo e bater cada valor.

## Fase 2 — MVP de leitura (`cofre-app`) 🟡 (funcional e testado; falta revisão visual)
Meta: usar no dia a dia para **consultar** senhas.

- [x] **Login**: bitwarden.com / bitwarden.eu / servidor próprio, e-mail, senha mestra (com mostrar/ocultar)
  - [x] 2FA por autenticador, e-mail (botão "enviar código") e YubiKey; "lembrar este dispositivo"
  - [x] Código de verificação de novo dispositivo
  - [x] Identidade do dispositivo, servidor e último e-mail salvos em `%APPDATA%\Cofre\settings.json` (nada secreto)
- [x] **Desbloqueio offline** com a senha mestra (o sync cifrado fica em memória; o decifrado é descartado ao bloquear)
- [x] **Lista virtualizada** com busca (nome, usuário, URI) e navegação por teclado (`List` do gpui-kit)
- [x] **Filtros**: todos, favoritos, por tipo, por organização, por pasta, sem pasta, lixeira — com contagem
- [x] **Detalhe do item**: todos os tipos, campos customizados, notas; segredos mascarados com mostrar/ocultar
- [x] **Copiar** com limpeza automática da área de transferência em 30 s (só se o conteúdo não mudou; sobrevive ao bloqueio)
- [x] **Bloqueio manual** e **automático** após 5 min sem teclado/mouse
- [x] **Sincronizar** (renova o token e baixa de novo, mantendo o item aberto)
- [x] Rede e KDF fora da thread de UI (Tokio + `spawn_blocking`); a UI nunca trava
- [x] **Testes de UI headless** (`crates/cofre-app/tests/ui.rs`): validação do login, filtros, busca, revelar,
      copiar + limpeza, bloquear, senha errada/certa, bloqueio por inatividade; e login completo pela UI
      contra o Vaultwarden local (`--ignored`)
- [x] **Revisão visual em janela real** — capturas da janela de verdade (claro/escuro, detalhe, login, configurações)
- [x] **Identidade visual própria**: neutros quentes, uma cor de destaque, ícones Lucide em "chips", seções na barra lateral
- [x] **Tela de configurações** (`Ctrl+,`, Esc fecha): tema Sistema/Claro/Escuro (segue o Windows ao vivo), 6 cores de
      destaque, tamanho do texto e cantos — aplicados na hora e salvos em `settings.json`
- [ ] Barra de título própria — a nativa do Windows não acompanha o tema do app
- [ ] Sessão expirada no "Sincronizar" → voltar para o login automaticamente
- [ ] Atalhos: `Ctrl+L` bloquear, `Ctrl+F` focar a busca, `Ctrl+C` copiar senha do item selecionado

## Fase 3 — Persistência segura e conveniência
- **Re-prompt** da senha mestra em itens marcados (hoje só mostra um aviso).
- Excluir o que é copiado do **histórico da área de transferência do Windows** (Win+V) e da nuvem.
- Bloquear ao **suspender/bloquear a sessão do Windows** e ao minimizar (opcional).
- **Refresh token** no cofre do SO via `keyring` (Windows Credential Manager).
- **Cache local cifrado** do sync (os dados já vêm cifrados do servidor — guardar o JSON cru, decifrar só na memória) → abrir offline.
- **Sync incremental**/periódico e via notificações (`/notifications/hub`, SignalR) — opcional.
- **Código TOTP ao vivo** para itens com chave de autenticador (`totp-rs`), com contador visual (hoje o detalhe mostra só a chave).
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
