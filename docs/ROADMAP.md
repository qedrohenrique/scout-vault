# Roadmap — Scout Vault

Cliente desktop nativo (Rust + GPUI) para servidores compatíveis com Bitwarden
(Bitwarden oficial e Vaultwarden). Objetivo: um app leve e rápido que substitua
o desktop Electron no uso diário — buscar, copiar e editar itens do cofre.

**Princípios**
- Nada decifrado vai para o disco. Segredos vivem em memória com `zeroize`.
- `scout-vault-core` não conhece UI: todo o protocolo e a criptografia são testáveis sozinhos.
- Desenvolvimento sempre contra um **Vaultwarden local** (`dev/docker-compose.yml`), nunca contra o cofre real.
- Referências de implementação: [`rbw`](https://github.com/doy/rbw) (MIT) e a documentação de segurança do Bitwarden.
  Não copiar código do SDK oficial (licença restritiva).

---

## Fase 0 — Fundação ✅ (esqueleto pronto)
- [x] Workspace `scout-vault-core` + `scout-vault-app`
- [x] Janela GPUI abrindo (gpui-kit)
- [x] Parse de `EncString` (tipos 2 e 4) e parâmetros de KDF, com testes
- [x] Vaultwarden local via Docker
- [x] CI (Windows): fmt, clippy, test
- [x] Instalar toolchain e validar `cargo run` localmente

## Fase 1 — Núcleo: login e leitura do cofre (`scout-vault-core`) ✅ (validado no Vaultwarden local)
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
- [x] Validar contra o Bitwarden oficial com uma conta real — roteiro em [TESTE-CONTA-REAL.md](TESTE-CONTA-REAL.md)
  - [x] O servidor oficial recusa clientes antigos ("Please update your app"): a versão anunciada
        (`Bitwarden-Client-Version`) precisa acompanhar a do app desktop oficial
  - [x] Protocolo revisado contra o código do servidor oficial e do rbw: token de sessão do 2FA por e-mail, `Device-Identifier`, `device_error`, captcha, código de 2FA inválido
  - [x] Guardar o token de "lembrar dispositivo" no Gerenciador de Credenciais do Windows
  - [x] SSO da empresa (com senha mestra), FIDO2/Windows Hello, Duo e criptografia v2 (COSE)
- [ ] Novos tipos de item do servidor: 6 = conta bancária, 7 = CNH, 8 = passaporte (hoje aparecem como `Unknown`)

**Testes:** `tools/gen_fixtures.py` gera, com a biblioteca `cryptography` do Python (implementação
independente), um cofre sintético completo — KDFs, user key, RSA, organização, chave de item,
PascalCase, item adulterado. `tests/fixtures.rs` precisa abrir tudo e bater cada valor.

## Fase 2 — MVP de leitura (`scout-vault-app`) 🟡 (funcional e testado; falta revisão visual)
Meta: usar no dia a dia para **consultar** senhas.

- [x] **Login**: bitwarden.com / bitwarden.eu / servidor próprio, e-mail, senha mestra (com mostrar/ocultar)
  - [x] 2FA por autenticador, e-mail (botão "enviar código") e YubiKey; "lembrar este dispositivo"
  - [x] Código de verificação de novo dispositivo
  - [x] Identidade do dispositivo, servidor e último e-mail salvos em `%APPDATA%\Scout Vault\settings.json` (nada secreto)
- [x] **Desbloqueio offline** com a senha mestra (o sync cifrado fica em memória; o decifrado é descartado ao bloquear)
- [x] **Lista virtualizada** com busca (nome, usuário, URI) e navegação por teclado (`List` do gpui-kit)
- [x] **Filtros**: todos, favoritos, por tipo, por organização, por pasta, sem pasta, lixeira — com contagem
- [x] **Detalhe do item**: todos os tipos, campos customizados, notas; segredos mascarados com mostrar/ocultar
- [x] **Copiar** com limpeza automática da área de transferência em 30 s (só se o conteúdo não mudou; sobrevive ao bloqueio)
- [x] **Bloqueio manual** e **automático** após 5 min sem teclado/mouse
- [x] **Sincronizar** (renova o token e baixa de novo, mantendo o item aberto)
- [x] Rede e KDF fora da thread de UI (Tokio + `spawn_blocking`); a UI nunca trava
- [x] **Testes de UI headless** (`crates/scout-vault-app/tests/ui.rs`): validação do login, filtros, busca, revelar,
      copiar + limpeza, bloquear, senha errada/certa, bloqueio por inatividade; e login completo pela UI
      contra o Vaultwarden local (`--ignored`)
- [x] **Revisão visual em janela real** — capturas da janela de verdade (claro/escuro, detalhe, login, configurações)
- [x] **Identidade visual própria**: neutros quentes, uma cor de destaque, ícones Lucide em "chips", seções na barra lateral
- [x] **Tela de configurações** (`Ctrl+,`, Esc fecha): tema Sistema/Claro/Escuro (segue o Windows ao vivo), 6 cores de
      destaque, tamanho do texto e cantos — aplicados na hora e salvos em `settings.json`
- [ ] Barra de título própria — a nativa do Windows não acompanha o tema do app
- [x] Sessão expirada no "Sincronizar" → voltar para o login automaticamente
- [x] Atalhos: `Ctrl+L` bloquear, `Ctrl+F` focar a busca, `Ctrl+U`/`Ctrl+P`/`Ctrl+T` copiar
      usuário/senha/código (no lugar de `Ctrl+C`, que segue copiando texto selecionado)

## Fase 3 — Persistência segura e conveniência
- **Re-prompt** da senha mestra em itens marcados (hoje só mostra um aviso).
- [x] Excluir o que é copiado do **histórico da área de transferência do Windows** (Win+V) e da nuvem.
- Bloquear ao **suspender/bloquear a sessão do Windows** e ao minimizar (opcional).
- [x] **Refresh token** no cofre do SO via `keyring` (Windows Credential Manager).
- [x] **Cache local cifrado** do sync (`sessao.json`, o JSON cru do servidor) → abrir offline e
      ir direto ao desbloqueio ("Lembrar sessão").
- **Sync incremental**/periódico e via notificações (`/notifications/hub`, SignalR) — opcional.
- [x] **Código TOTP ao vivo** (RFC 6238 implementada no `scout-vault-core`, validada com os vetores da RFC;
      `otpauth://`, base32 e Steam Guard), com segundos restantes.
- [x] **Gerador de senhas** (`Ctrl+G`).
- **Gerador de passphrases** — precisa de uma lista de palavras (ex.: EFF, CC BY 3.0).
- **Atalho global** (ex.: `Ctrl+Shift+L`) abrindo uma janela de busca rápida estilo launcher.
- Desbloqueio com **Windows Hello** (pesquisa: chave protegida por TPM/DPAPI).

## Fase 4 — Escrita
- [x] Cifrar: AES-256-CBC + HMAC com IV aleatório e, em contas v2, COSE XChaCha20-Poly1305 (com `kid`).
- [x] Criar / editar / lixeira / restaurar / apagar de vez (`POST/PUT /api/ciphers`, `…/delete`,
      `…/restore`, `DELETE`), com `encryptedFor` e `lastKnownRevisionDate`.
- [x] Edição **preserva o que o Scout Vault não conhece** (passkeys, regras de URI, campos vinculados…)
      e recalcula o `uriChecksum` das URIs.
- [x] Histórico de senhas do item (até 5).
- [x] Testes ao vivo no Vaultwarden (núcleo e interface).
- [x] Pastas: criar, renomear, apagar (nome cifrado com a user key).
- Criar itens direto numa organização (precisa escolher coleções).
- Testes de ida e volta com o cofre web oficial (ler no web vault o que o Scout Vault gravou).

## Fase 4½ — Integração com o Windows e o navegador
- [x] **Preencher em apps do Windows** ("auto-type", como no KeePass): `Ctrl+Alt+A` digita
  usuário ⇥ senha ↵ na janela em foco (`SendInput` Unicode), escolhendo o item pelo título;
  lista para escolher quando há dúvida; pedido guardado até desbloquear; para de digitar se o
  foco mudar. Pendências: sequência personalizada por item (`{USERNAME}{TAB}…`), `{TOTP}`,
  janelas que rodam como administrador (o Windows bloqueia a digitação vinda de app comum).
- [x] **Preencher no navegador**: extensão MV3 própria (`extension/`) para Chrome/Edge/Brave,
  *native messaging* com o próprio `scout-vault.exe` como ponte e pipe local com ACL do usuário.
  Pendências: Firefox, salvar login novo pela extensão, preencher sozinho ao abrir a página,
  publicar na loja (hoje carrega sem compactação).
- **Login social (Google etc.)**: o Bitwarden não oferece "entrar com Google" para contas
  pessoais; só existe via **SSO de organização** (Google Workspace como provedor), que o Scout Vault
  já suporta. Nada a fazer até o servidor oferecer.

- [x] **Bandeja do Windows**: fechar esconde a janela e deixa o app rodando; menu Abrir/Bloquear/
  Sair; uma instância só (abrir de novo mostra a existente).

## Fase 5 — Recursos avançados
- Organizações e coleções (escrita), compartilhamento de item.
- [x] Anexos: anexar, abrir, salvar e remover (chave própria por anexo; envio direto ou Azure).
      O envio pela nuvem oficial (Azure) ainda não foi testado com uma conta real.
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
