# Roadmap — Scout Vault

Cliente desktop nativo (Rust + GPUI) para servidores compatíveis com Bitwarden
(Bitwarden oficial e Vaultwarden). Objetivo: um app **leve, rápido e agradável** que substitua o
desktop Electron no uso diário — e que vá além dele onde um app nativo de desktop brilha
(teclado, Windows, privacidade local).

**Princípios**
- Nada decifrado vai para o disco (exceção explícita e opcional: a user key no Gerenciador de
  Credenciais para "abrir sem senha por N dias"; com Windows Hello ela fica cifrada pelo Hello).
  Segredos vivem em memória com `zeroize`.
- `scout-vault-core` não conhece UI: protocolo e criptografia testáveis sozinhos.
- Desenvolvimento contra um **Vaultwarden local** (`dev/`); conta real só para validação manual.
- Implementar a partir das especificações e de projetos MIT (ex.: [`rbw`](https://github.com/doy/rbw));
  **não copiar código do SDK oficial** (licença restritiva).
- Interoperar sem perder dados: o que o Scout Vault não entende volta intacto ao servidor.

---

## Onde estamos (0.2.x)

| Área | Feito |
|---|---|
| **Conta** | Login (PBKDF2/Argon2id), 2FA (autenticador, e-mail, YubiKey, FIDO2/Windows Hello, Duo), novo dispositivo, "lembrar dispositivo", SSO com senha mestra, contas v2 (COSE), captcha/limite de tentativas explicados |
| **Sessão** | Sessão lembrada e cache cifrado (abre offline), abrir sem senha por N dias, login completo a cada N dias, desbloqueio com Windows Hello, sync em segundo plano, bloqueio por inatividade configurável, bloqueio junto com o Windows (Win+L e suspensão), troca de senha detectada |
| **Cofre** | Todos os tipos de item, busca, filtros, pastas (CRUD), organizações e coleções (criar/escolher/trocar), acesso rápido (itens fixados), TOTP ao vivo, lixeira, anexos (enviar/abrir/salvar, arrastar e soltar), histórico de senha preservado |
| **Escrita** | Criar/editar/lixeira/restaurar/apagar, preservando passkeys, regras de URI e campos desconhecidos |
| **Windows** | Preencher em apps (`Ctrl+Alt+A`), bandeja (fechar ≠ sair), instância única, área de transferência fora do Win+V, ícone/recursos do executável |
| **Navegador** | Extensão MV3 (Chrome/Edge/Brave) com native messaging e pipe restrito ao usuário |
| **Interface** | Tema claro/escuro, cor de destaque, animações (com "Parado"), toasts, gerador de senhas, atalhos |
| **Qualidade** | Testes de UI headless (inclusive de layout), vetores públicos de cripto, testes ao vivo no Vaultwarden |

Validação manual com conta real: roteiro em [TESTE-CONTA-REAL.md](TESTE-CONTA-REAL.md).

---

## Comparativo com o mercado

Visão geral (nov/2026) a partir da documentação pública de cada produto, olhando só o **app de
desktop + extensão** no Windows. Recursos variam por plano; ✅ tem, 🟡 parcial, ❌ não tem.

| Recurso | Scout Vault | Bitwarden (oficial) | 1Password | KeePassXC | Proton Pass |
|---|:-:|:-:|:-:|:-:|:-:|
| App nativo leve (sem Electron) | ✅ | ❌ | ❌ | ✅ | ❌ |
| Sincronização na nuvem / multi-dispositivo | ✅ (servidor Bitwarden) | ✅ | ✅ | ❌ (arquivo) | ✅ |
| Preenchimento no navegador | 🟡 popup | ✅ inline + automático | ✅ | ✅ | ✅ |
| Salvar/atualizar login pelo navegador | ✅ | ✅ | ✅ | ✅ | ✅ |
| Preencher em apps do desktop (auto-type) | ✅ | ❌ | 🟡 | ✅ | ❌ |
| Passkeys (criar e usar) | ❌ (preserva) | ✅ | ✅ | ✅ | ✅ |
| Busca rápida global (atalho no sistema) | ❌ | ❌ | ✅ Quick Access | 🟡 | ❌ |
| Desbloquear com Windows Hello | ✅ | ✅ | ✅ | ✅ | ✅ |
| Bloquear junto com o Windows | ✅ | ✅ | ✅ | ✅ | ✅ |
| Re-prompt da senha em itens sensíveis | ✅ | ✅ | 🟡 | ❌ | ❌ |
| Saúde das senhas (fracas, repetidas, vazadas) | ❌ | ✅ relatórios | ✅ Watchtower | ✅ | ✅ |
| Gerador de passphrase e de usuário | 🟡 só senha | ✅ | ✅ | ✅ | ✅ |
| Aliases de e-mail (esconder e-mail) | ❌ | ✅ via serviços | ✅ | ❌ | ✅ nativo |
| Compartilhamento seguro (Send / link) | ❌ | ✅ Send | ✅ | ❌ | ✅ |
| Mover item para organização / compartilhar | ❌ | ✅ | ✅ | ❌ | ✅ |
| Histórico de versões do item | 🟡 só senha | 🟡 só senha | ✅ | ✅ | ❌ |
| Várias contas ao mesmo tempo | ❌ | ✅ | ✅ | ✅ (vários arquivos) | ✅ |
| Agente SSH | ❌ (guarda chaves) | ✅ | ✅ | ✅ | ❌ |
| Importar / exportar | ❌ | ✅ | ✅ | ✅ | ✅ |
| Acesso de emergência | ❌ | ✅ | 🟡 | ❌ | ❌ |
| Ícones dos sites na lista | ❌ | ✅ | ✅ | ✅ | ✅ |
| Firefox | ❌ | ✅ | ✅ | ✅ | ✅ |
| Instalador / atualização automática | ❌ (zip) | ✅ | ✅ | ✅ | ✅ |
| Linux / macOS | ❌ | ✅ | ✅ | ✅ | ✅ |
| Auditoria de segurança externa | ❌ | ✅ | ✅ | ✅ | ✅ |

**Onde já nos destacamos:** leveza e rapidez, auto-type de verdade, bandeja/instância única,
área de transferência fora do histórico, interface animada e acessível pelo teclado, Vaultwarden
como cidadão de primeira classe.

**Lacunas que mais pesam no dia a dia:** preenchimento inline no navegador, passkeys, saúde das
senhas, instalador (Windows Hello, bloqueio com o Windows, re-prompt e salvar logins já entraram).

---

## Próximas fases (prioridade de cima para baixo)

### Fase A — Segurança do dia a dia
- ~~Desbloquear com Windows Hello~~ — feito: a user key fica cifrada por uma assinatura do Hello (chave no TPM), com a senha mestra como alternativa.
- ~~Bloquear ao bloquear/suspender o Windows~~ — feito (opção "Bloquear junto com o Windows", ligada por padrão).
- ~~Re-prompt da senha mestra~~ — feito: itens marcados pedem a senha para revelar, copiar, editar e digitar; o navegador não recebe esses itens.
- Bloquear ao minimizar (opcional) e "limpar área de transferência" com tempo configurável.
- ~~Sync em segundo plano~~ — feito: a cada minuto o app consulta `accounts/revision-date` (barato) e sincroniza só se o cofre mudou. Próximo passo: notificações em tempo real (`/notifications/hub`, SignalR via WebSocket).

### Fase B — Navegador de primeira linha
- ~~Salvar e atualizar logins~~ — feito: barra "Salvar/Atualizar no Scout Vault?" depois do envio do formulário; o app decide entre novo e atualização.
- **Menu inline** no campo (ícone do Scout Vault dentro do input) e atalho de teclado na página.
- Preenchimento de **cartões e identidades**; copiar TOTP automaticamente após preencher.
- **Firefox** (manifest próprio, `allowed_extensions`).
- Publicar nas lojas (Chrome Web Store, Edge Add-ons) para não precisar do modo desenvolvedor.

### Fase C — Produtividade de desktop
- **Busca rápida global** (`Ctrl+Shift+Espaço`): janela flutuante para achar, copiar e preencher sem abrir o app.
- **Ícones dos sites** na lista (serviço de ícones do servidor, com cache e opção de desligar).
- Auto-type: **sequência por item** (`{USERNAME}{TAB}{PASSWORD}{ENTER}`, `{TOTP}`, atrasos) e janelas de admin.
- Atalhos configuráveis; paleta de comandos (`Ctrl+K`).
- Barra de título própria que acompanha o tema.

### Fase D — Saúde e geração
- **Relatório de saúde**: senhas fracas, repetidas, antigas e **vazadas** (HIBP por k-anonymity — só um
  prefixo do hash sai da máquina); sites que aceitam 2FA e o item não tem.
- **Passphrases** (lista EFF, CC BY 3.0) e **gerador de usuário**.
- **Aliases de e-mail** via SimpleLogin, addy.io, Fastmail e DuckDuckGo (como o Bitwarden faz).

### Fase E — Passkeys
- Guardar e usar **passkeys** pela extensão (WebAuthn com credenciais do cofre, formato `fido2Credentials`).
- Mais tarde: provedor de passkeys do Windows (API de plugins do Windows 11), se estável.

### Fase F — Compartilhar e colaborar
- **Mover item para organização** (recifrar com a chave da org, inclusive anexos).
- **Bitwarden Send** (texto e arquivo, com prazo, senha e limite de acessos).
- **Várias contas** abertas e troca rápida entre elas.
- Acesso de emergência (aceitar/conceder) — depende do suporte do servidor.
- Novos tipos de item do servidor (conta bancária, CNH, passaporte) quando estáveis.

### Fase G — Dados
- **Importar**: Bitwarden JSON/CSV, 1Password `.1pux`, KeePass `.kdbx`/CSV, Chrome/Edge CSV, Proton Pass.
- **Exportar**: JSON cifrado com senha (formato do Bitwarden) e CSV (com aviso).
- Histórico de versões do item (local, com restauração).

### Fase H — Distribuição e confiança
- **Instalador** (MSI/winget), **atualização automática** com assinatura das atualizações.
- **Assinatura de código** do executável (remove o aviso do SmartScreen).
- Builds para **Linux** e **macOS** (GPUI já roda nelas).
- **Revisão de segurança externa** e política de divulgação de falhas.
- Interface em inglês (i18n) e revisão de acessibilidade (leitor de tela, contraste).
- Agente SSH (o cofre já guarda as chaves).

---

## Riscos e decisões em aberto
- **API não documentada formalmente** — o servidor muda requisitos (ex.: versão mínima de
  cliente, `encryptedFor`); acompanhar `rbw`, o changelog do Vaultwarden e os clientes oficiais.
- **Envio de anexos para a nuvem oficial (Azure)** ainda não validado com conta real.
- **Revisão de segurança** — antes de recomendar para uso amplo.
- **Marca** — não usar nome/logo "Bitwarden"; "compatível com Bitwarden", projeto não oficial.
- **"Abrir sem senha por N dias"** troca conveniência por risco local; fica desligado por padrão
  e é descartado quando a senha muda.
