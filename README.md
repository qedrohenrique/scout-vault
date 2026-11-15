<p align="center"><img src="docs/imagens/logo.png" width="112" alt="Logo do Scout Vault: um cadeado de chapéu de explorador"></p>

# Scout Vault

Cliente desktop nativo, em **Rust + GPUI**, para servidores compatíveis com Bitwarden
(Bitwarden oficial e Vaultwarden). A ideia é ter um app leve e rápido para o dia a dia:
abrir, buscar, copiar e fechar, sem o peso de um app Electron.

> Projeto não oficial, sem afiliação com a Bitwarden Inc. **Em desenvolvimento — não use com seu cofre real ainda.**

![Scout Vault no tema escuro, com um item aberto](docs/imagens/scout-vault-escuro.png)

---

## Sumário
- [O que já funciona](#o-que-já-funciona)
- [Aparência e configurações](#aparência-e-configurações)
- [Atalhos](#atalhos)
- [Segurança](#segurança)
- [Arquitetura](#arquitetura)
- [Pré-requisitos (Windows)](#pré-requisitos-windows)
- [Rodando](#rodando)
- [Servidor de desenvolvimento](#servidor-de-desenvolvimento)
- [Testes](#testes)
- [Como acrescentar uma configuração](#como-acrescentar-uma-configuração)
- [Novidades e releases](#novidades)
- [Roadmap](#roadmap)

---

## O que já funciona

**Entrar**
- Servidores bitwarden.com, bitwarden.eu ou um servidor próprio (Vaultwarden).
- Senha mestra com KDF PBKDF2-SHA256 ou Argon2id. A senha nunca sai da máquina: só vai um hash derivado dela.
- Verificação em duas etapas: autenticador, código por e-mail, YubiKey, chave de segurança
  FIDO2/Windows Hello (API nativa do Windows) e Duo. "Lembrar este dispositivo" guarda o token no
  Gerenciador de Credenciais do Windows.
- Login por **SSO da empresa** (navegador + senha mestra, como o CLI oficial).
- Contas com a **criptografia v2** do Bitwarden (COSE: XChaCha20-Poly1305, XAES-256-GCM).
- Código de verificação de novo dispositivo.
- O servidor, o último e-mail e a identidade do dispositivo ficam salvos para a próxima vez.

**Usar o cofre**
- Lista virtualizada com busca por nome, usuário e endereço, navegável pelo teclado.
- Filtros na barra lateral, cada um com contagem: todos os itens, favoritos, por tipo (logins, cartões,
  identidades, notas, chaves SSH), por organização, por pasta, sem pasta e lixeira.
- O detalhe do item mostra todos os campos, inclusive os customizados e as notas. Os segredos começam
  mascarados e podem ser revelados um a um.
- **Código do autenticador (TOTP) ao vivo**, com os segundos restantes. Aceita chave base32,
  URI `otpauth://` (SHA-1/256/512, dígitos e período próprios) e `steam://` (Steam Guard).
- Ao copiar um segredo, a área de transferência é limpa sozinha em 30 s. Ela só é limpa se você
  não tiver copiado outra coisa depois. No Windows, o que o Scout Vault copia **não entra no histórico
  (Win+V) nem na área de transferência na nuvem**.
- **Gerador de senhas** (`Ctrl+G` ou o ícone de dados): tamanho, conjuntos de caracteres e opção
  de evitar caracteres parecidos. As opções ficam salvas; a senha gerada, não.
- **Preencher em outros apps** (`Ctrl+Alt+A`, em qualquer janela): o Scout Vault escolhe o login pelo
  título da janela (nome do item ou site) e **digita** `usuário ⇥ senha ↵` nela, sem passar pela
  área de transferência. Se o título não decidir, mostra a lista para escolher; com o cofre
  bloqueado, pede a senha e digita depois. Liga/desliga (e o Enter final) nas configurações.
- Sincronizar baixa o cofre de novo sem fechar o item que está aberto. Se a sessão tiver expirado
  no servidor, o app volta para o login avisando.

**Editar** (cifrado na sua máquina antes de sair dela)
- **Novo item** (`+` na lista ou `Ctrl+N`): login, nota, cartão ou identidade. **Editar**
  (`Ctrl+E`) qualquer item, inclusive chaves SSH: nome, campos, sites, campos personalizados,
  notas, pasta e favorito. Botão para gerar a senha ali mesmo.
- Ao trocar a senha, a antiga vai para o **histórico de senhas** do item (como nos apps oficiais).
- O Scout Vault só troca o que você editou: passkeys, regras de URI, campos vinculados e outros dados
  criados por outros clientes voltam intactos ao servidor.
- **Lixeira**: mover, restaurar e **apagar de vez** (pede confirmação).
- **Acesso rápido**: o alfinete no detalhe de um item o fixa numa seção no topo da barra lateral
  (até 10). Clicar abre o item direto, mesmo com outro filtro ativo. Fica salvo no computador.
- **Organizações**: ao criar um item, escolha o dono ("Meu cofre" ou a organização) e as
  coleções em que ele fica (só as que você pode editar). Itens existentes da organização podem
  trocar de coleção. As coleções aparecem embaixo de cada organização na barra lateral; criar um
  item com uma coleção aberta já o põe nela.
- **Pastas**: `+` ao lado de "Pastas" na barra lateral cria uma; com a pasta aberta, o cabeçalho
  da lista tem renomear e apagar (os itens dela vão para "Sem pasta"). O nome vai cifrado.
  **Arraste** um item da lista até uma pasta (ou "Sem pasta") para movê-lo, e uma pasta até
  outra para virar subpasta (no Bitwarden é o nome "Pai/Filha"; soltar no título "Pastas" a
  devolve ao topo). Subpastas aparecem recuadas.
- **Anexos**: no formulário de um item novo (ou ao editar) e no detalhe do item há uma área para
  **clicar ou arrastar arquivos** do Explorer (até 500 MB cada; vários de uma vez). Dá para abrir
  com o programa padrão, salvar onde quiser e remover. Cada anexo tem chave própria; a cópia aberta
  fica em `%TEMP%\ScoutVault-anexos` e é apagada ao bloquear. Na conta gratuita do bitwarden.com
  anexos exigem **Premium** — o app avisa com essa mensagem.
- Itens de organização em que você só tem leitura não mostram Editar.

**Preencher no navegador** (Chrome, Edge, Brave)
- Extensão própria na pasta [`extension/`](extension/). Ligue em Configurações → Navegador e
  siga as instruções que aparecem (carregar a extensão sem compactação).
- Num site, clique no ícone do Scout Vault ou use `Ctrl+Shift+L`: aparecem os logins daquele site
  (domínio base, como no Bitwarden). Clique num para preencher usuário e senha.
- A senha só sai do app quando você clica, e só para um item com endereço do mesmo site.
  A extensão fala com o app por *native messaging* e um canal local que só o seu usuário do
  Windows acessa. Com o cofre bloqueado, o app aparece pedindo a senha.
- **Salvar logins:** ao entrar num site com um login que o cofre ainda não tem (ou com uma senha
  nova), aparece no canto da página "Salvar este login no Scout Vault?" (ou "Atualizar a senha
  de …?"). Nada é salvo sem o clique; a senha antiga vai para o histórico do item. Com o cofre
  bloqueado, o app pede a senha e salva em seguida. Para isso a extensão roda em todos os sites
  (o navegador avisa ao instalar); ela só lê o formulário quando ele é enviado.

**Bandeja do Windows**
- Fechar a janela (X ou Alt+F4) deixa o app rodando no ícone perto do relógio: `Ctrl+Alt+A` e o
  preenchimento no navegador continuam funcionando. Clique no ícone para abrir. O botão direito
  abre o menu do próprio app (com o tema e a cor de destaque, ícones e animação), com **Abrir**, o **Acesso rápido** (copiar usuário, senha ou código dos itens
  fixados sem abrir a janela), **Gerar e copiar senha**, **Sincronizar agora**,
  **Bloquear**/**Desbloquear…** e **Sair**. Abrir o executável de novo só mostra a janela que já
  existe. Dá para desligar em Configurações → Sessão.

**Sessão e bloqueio**
- **Lembrar sessão** (ligado por padrão): ao abrir o app, ele vai direto para o desbloqueio, só
  com a senha mestra, e **abre mesmo sem internet**. Guarda o cofre **cifrado** como veio do
  servidor em `%APPDATA%\Scout Vault\sessao.json` e o refresh token no Gerenciador de Credenciais do
  Windows. "Sair da conta" ou desligar a opção apagam os dois.
- **Quanto tempo ficar conectado** (Configurações → Sessão):
  - *Pedir a senha mestra ao abrir*: sempre, ou lembrar por 1, 7 ou 30 dias. Nesse período o app
    abre já desbloqueado; a chave fica no Gerenciador de Credenciais, vence no prazo e é
    descartada se a senha mestra for trocada (o app percebe no sync).
  - *Pedir login completo a cada*: 7, 30 (padrão) ou 90 dias, ou nunca.
  - *Bloquear sem uso após*: 1, 5 (padrão), 15 ou 60 minutos, ou nunca.
  - *Sincronizar em segundo plano* (ligado por padrão): a cada minuto o app pergunta ao servidor
    se o cofre mudou (só a data da última mudança, sem baixar nada) e, se mudou em outro
    aparelho, sincroniza sozinho e avisa. Com um formulário aberto, espera.
  - *Desbloquear com Windows Hello* (desligado por padrão): rosto, digital ou PIN no lugar da
    senha mestra. O Windows Hello assina um desafio com uma chave que não sai do TPM; essa
    assinatura cifra a user key, e só a versão cifrada fica no Gerenciador de Credenciais. Trocar
    a senha mestra ou sair da conta apaga a configuração.
  - *Bloquear junto com o Windows* (ligado por padrão): ao bloquear o computador (Win+L) ou ao
    suspender/hibernar, o cofre bloqueia também.
- Bloqueia pelo botão de cadeado, por `Ctrl+L`, pela bandeja ou sozinho, sem uso.
- Ao bloquear, o cofre decifrado sai da memória. Para desbloquear basta a senha mestra, sem rede.
- **Pedir a senha mestra** (opção do item, como no Bitwarden): mostrar, copiar ou editar os dados
  secretos e digitar com `Ctrl+Alt+A` pedem a senha de novo; a liberação vale até trocar de item.
  A conferência é local (sem rede). O navegador não preenche esses itens.

## Aparência e configurações

O Scout Vault tem um visual próprio e minimalista. Os fundos são neutros quentes (um tom de papel no
claro, grafite profundo no escuro), e há **uma única cor de destaque**, usada só onde importa:
seleção, foco, botão principal, favoritos e a marca. Os ícones são do [Lucide](https://lucide.dev)
e aparecem dentro de quadradinhos suaves.

| Claro | Escuro |
|---|---|
| ![Tema claro](docs/imagens/scout-vault-claro.png) | ![Tema escuro](docs/imagens/scout-vault-escuro.png) |

As telas de **login e desbloqueio** têm uma arte vetorial animada ao fundo: faixas onduladas nas
laterais que balançam devagar e pequenas formas fechadas (bolinhas, círculos, triângulos,
quadrados, losangos, pentágonos e hexágonos) subindo e girando devagar pelo meio, tudo em traço
fino na cor de destaque. A marca grande fica centralizada sobre o formulário. É desenhada pelo
próprio app a cada quadro, sem imagens, e só se mexe com a janela em foco.

A interface também tem **animações curtas** (até ~¼ s, só opacidade e posição): telas e painéis
entram suaves, a barra lateral entra em cascata, o filtro escolhido acende, a seleção da lista
envolve o ícone do item, mensagens de erro dão uma leve sacudida, "Entrando…" pulsa, uma barra
fina corre enquanto sincroniza ou salva, o ícone de sincronizar gira e vira ✓ e o botão de copiar
vira ✓. Avisos ("Copiado", "Item criado", erros) aparecem como **toasts** que sobem do pé da
janela, ficam alguns segundos e descem sumindo. Ao trocar o tema, a cor de destaque, o texto ou os cantos, a interface se transforma no lugar: cada
cor vai da antiga à nova (nada some), e entre claro e escuro uma onda sai do botão clicado. **Configurações → Movimento → Parado** desliga tudo isso e a arte.

![Tela de entrada com a arte de fundo](docs/imagens/fundo-animado-previa.png)

A **logo** é um cadeado usando um chapéu de explorador. Ela é o ícone do executável (barra de
tarefas, Alt+Tab, Explorer) e aparece como glifo dentro do app, na cor de destaque escolhida. Tudo
sai de uma geometria só, em `tools/gen_logo.py`:
`python tools/gen_logo.py crates/scout-vault-app/assets/brand` gera de novo o SVG, os PNGs e o `.ico`.

A tela de **Configurações** abre com `Ctrl+,` ou pela engrenagem. A engrenagem fica na barra
lateral do cofre e também nas telas de login e desbloqueio.

![Tela de configurações](docs/imagens/configuracoes.png)

| Opção | Valores | Observação |
|---|---|---|
| Tema | Sistema · Claro · Escuro | "Sistema" acompanha o claro/escuro do Windows na hora |
| Cor de destaque | Jade · Índigo · Latão · Coral · Ameixa · Grafite | Tons escolhidos para o texto do botão principal ter contraste AA |
| Tamanho do texto | Pequeno (14) · Padrão (15) · Grande (17) | Vale para todas as telas |
| Cantos | Retos · Suaves · Redondos | Arredondamento de botões, campos e cartões |

As mudanças valem na hora, sem reiniciar o app. Elas ficam salvas em `settings.json` (veja
[Segurança](#segurança)). Abrir as configurações não fecha o cofre: ele continua por baixo, e
o bloqueio automático segue valendo.

![Tela de login](docs/imagens/login.png)

## Atalhos

| Atalho | Onde | Ação |
|---|---|---|
| `Ctrl+,` | qualquer tela | Abre/fecha as configurações |
| `Ctrl+Alt+A` | **qualquer app do Windows** | Preenche usuário e senha na janela em foco |
| `Ctrl+G` | qualquer tela | Abre/fecha o gerador de senhas |
| `Esc` | configurações, gerador, lista de preenchimento | Fecha |
| `Ctrl+F` | cofre | Vai para a busca |
| `Ctrl+N` | cofre | Novo item |
| `Ctrl+E` | cofre | Edita o item aberto |
| `Ctrl+U` | cofre | Copia o usuário do item aberto |
| `Ctrl+P` | cofre | Copia a senha do item aberto |
| `Ctrl+T` | cofre | Copia o código do autenticador do item aberto |
| `Ctrl+L` | cofre | Bloqueia |
| `↑` `↓` | lista | Move a seleção |
| `Enter` | lista | Abre o item |

A cópia usa `Ctrl+U`/`Ctrl+P`/`Ctrl+T` (como o app oficial) em vez de `Ctrl+C`, que continua
copiando o texto selecionado na busca.

## Segurança

Achou uma falha? Relate em particular, como explica o [SECURITY.md](SECURITY.md).

- **Nada decifrado vai para o disco.** Os segredos vivem em memória dentro de `Secret`/`Zeroizing`
  e são apagados quando liberados. O `Debug` deles nunca mostra o conteúdo.
- O `settings.json` (`%APPDATA%\Scout Vault\settings.json` no Windows e `~/.config/scout-vault/settings.json` nos
  demais) guarda **só** o servidor, o último e-mail, a identidade do dispositivo e a aparência.
- A criptografia segue o esquema do Bitwarden. O HMAC é conferido em tempo constante **antes** de
  decifrar com AES-256-CBC. As chaves de organização usam RSA-OAEP, e cada item pode ter chave própria.
- Um item ou uma organização corrompidos não derrubam o resto do cofre: eles aparecem como erro
  isolado.
- Rede e KDF rodam fora da thread de UI, então a interface nunca trava.
- O desenvolvimento é sempre contra um **Vaultwarden local**, nunca contra um cofre real.

## Arquitetura

```
crates/
  scout-vault-core/      protocolo, criptografia e modelo do cofre — sem UI, testável sozinho
    api.rs         cliente HTTP: prelogin, login (2FA, novo dispositivo), refresh, sync
    crypto.rs      master key (PBKDF2/Argon2id), HKDF, AES-CBC + HMAC, RSA-OAEP
    enc_string.rs  formato EncString (2.iv|ct|mac, 4.data)
    sync.rs        resposta do sync, ainda cifrada
    vault.rs       cofre decifrado: itens, pastas, organizações, busca
  scout-vault-app/       interface GPUI (binário `cofre`), via gpui-kit
    app.rs         raiz da janela: troca de telas, bloqueio automático, configurações
    login.rs       login, 2FA, novo dispositivo
    unlock.rs      desbloqueio offline
    vault_view.rs  barra lateral, lista e detalhe
    item_list.rs   dados da lista: filtro e busca
    appearance.rs  paleta, temas claro/escuro, cor de destaque, texto e cantos
    settings_view.rs  tela de configurações
    ui.rs          peças visuais compartilhadas (marca, chips, controle segmentado)
    build.rs       embute o ícone (assets/brand/scout-vault.ico) no executável do Windows
    assets/brand/  logo colorida (SVG/PNG/ICO) e o glifo monocromático da marca
    settings.rs    preferências não sensíveis em disco
    flow.rs        fluxos de login/sync sem UI
dev/               Vaultwarden local (docker-compose)
tools/             gerador de fixtures, de conta de teste e da logo (Python)
docs/ROADMAP.md    plano por fases
```

**Como o tema funciona:** `appearance.rs` parte dos temas padrão do gpui-kit e troca as cores pela
paleta do Scout Vault (neutros de cada modo mais a cor de destaque). Depois ajusta tamanho da fonte e
raios e registra o resultado como os temas claro e escuro. `appearance::apply` faz isso e escolhe
o modo. O `ScoutVaultApp` chama essa função ao abrir e quando o Windows troca de claro para escuro (se
o tema for "Sistema"). A tela de configurações também chama a cada mudança.

## Pré-requisitos (Windows)
- Rust estável: `winget install Rustlang.Rustup`
- Visual Studio Build Tools com a carga "Desenvolvimento para desktop com C++" (linker MSVC + Windows SDK)
- Docker Desktop ou Docker no WSL (só para o servidor de desenvolvimento)
- Python 3 com `cryptography` (só para as ferramentas de `tools/`)

## Rodando
```bash
cargo run            # abre o app
cargo run --release  # build otimizado, sem console no Windows
```

Em builds de debug, o app pode entrar sozinho no Vaultwarden local com a conta de teste. Isso só
funciona com servidores em `localhost`:
```bash
set -a; . dev/test-account.env; set +a
SCOUT_VAULT_DEV_AUTOLOGIN=1 cargo run
```

## Servidor de desenvolvimento
O Vaultwarden local roda em `http://localhost:8000` e funciona com Docker Desktop ou com o Docker
instalado dentro do WSL (sem Docker Desktop):
```bash
wsl -d Ubuntu -u root -- bash -c "service docker start; cd /mnt/d/Repositories/cofre/dev && docker compose up -d"
```
O WSL desliga quando fica ocioso; mantenha um terminal do WSL aberto enquanto testa.

Para criar uma conta de teste já populada, sem navegador (as credenciais vão para
`dev/test-account.env`, que o git ignora):
```bash
python tools/dev_seed.py
```

Para testar itens de organização, crie nessa conta a organização "Empresa de teste", com as
coleções Financeiro e TI (só no servidor local; não faz nada se ela já existir):
```bash
python tools/dev_org.py
```

## Testes
```bash
cargo test --workspace                 # tudo (núcleo + UI headless)
cargo test -p scout-vault-core               # só o núcleo
cargo test -p scout-vault-app --test ui      # só a UI
```

- **Núcleo:** `tools/gen_fixtures.py` gera um cofre sintético completo usando a biblioteca
  `cryptography` do Python, uma implementação independente da nossa. O arquivo
  `tests/fixtures.rs` precisa abrir esse cofre e bater cada valor.
- **UI:** os testes rodam as telas reais numa janela headless, com cliques e digitação de verdade.
  Eles cobrem a validação do login, filtros, busca, revelar e copiar (com a limpeza da área de
  transferência), bloqueio manual e por inatividade, troca de tema, cor, texto e cantos, e a
  aparência restaurada ao reabrir.
- **Ao vivo** (precisa do Vaultwarden local):
  ```bash
  set -a; . dev/test-account.env; set +a
  cargo test -p scout-vault-core --test live_server -- --ignored --nocapture
  cargo test -p scout-vault-app --test ui -- --ignored
  ```
- **2FA ao vivo:** `python tools/dev_seed.py --totp` cria uma conta com autenticador
  (credenciais em `dev/test-account-2fa.env`) para `cargo test -p scout-vault-app --test live_2fa -- --ignored`.

A CI (GitHub Actions, em `.github/workflows/`) roda em cada push e PR:
- **CI**: `cargo fmt --check`; `cargo clippy -D warnings` e `cargo test --workspace` no Windows;
  os testes ao vivo do núcleo contra um Vaultwarden descartável (conta e organização criadas
  pelos scripts de `tools/`); e a sintaxe da extensão.
- **Segurança**: `cargo deny check` (vulnerabilidades do RustSec, licenças, crates e origens —
  regras em `deny.toml`) e CodeQL (Rust, JavaScript da extensão e os próprios workflows).
  Também roda toda segunda, para pegar falhas novas sem precisar de commit.
- **Dependabot** abre PRs semanais de dependências e actions.
Os testes headless não desenham pixels. Por isso, mudanças visuais precisam ser conferidas numa
janela real.

## Como acrescentar uma configuração
1. Acrescente o campo em `Appearance` (`appearance.rs`) com `#[serde(default)]`. Assim um
   `settings.json` antigo continua abrindo.
2. Se o campo mudar o tema, aplique-o em `appearance::build`. Se não mudar, leia-o onde for usado.
3. Acrescente uma linha em `SettingsView::render` (`settings_view.rs`) com `Self::row(...)`. O
   controle pode ser um `ui::segmented`, um `Switch` ou algo próprio. Para uma nova seção, use
   outro `Self::group(...)`.
4. Escreva um teste em `tests/ui.rs` que clique no controle e confira o efeito.

## Novidades
O que mudou em cada versão está no [CHANGELOG.md](CHANGELOG.md); os downloads ficam nas
[Releases](https://github.com/qedrohenrique/scout-vault/releases).

**Publicar uma versão** (o workflow `.github/workflows/release.yml` faz o resto):
1. No `CHANGELOG.md`, troque `## [Ainda não lançado]` por `## [X.Y.Z] - AAAA-MM-DD` e ajuste os
   links do fim do arquivo.
2. Ponha `X.Y.Z` em `[workspace.package]` no `Cargo.toml` (e em `extension/manifest.json`).
3. Confira as notas com `python tools/release_notes.py X.Y.Z`, faça o commit e envie a tag:
   `git tag vX.Y.Z` e `git push origin vX.Y.Z`.

O GitHub Actions roda os testes, compila, monta o zip (`tools/package_release.py`) com o
`.sha256` e cria a release com as notas daquela seção do changelog (versões 0.x saem como
pré-release). Se a tag não bater com o `Cargo.toml` ou faltar a seção, nada é publicado.

## Roadmap
Veja [docs/ROADMAP.md](docs/ROADMAP.md), que inclui um comparativo com Bitwarden, 1Password,
KeePassXC e Proton Pass. Windows Hello, bloqueio junto com o Windows e re-prompt já entraram; os próximos passos são
o preenchimento direto no campo do site, a busca rápida por atalho global e o relatório de saúde das senhas.
