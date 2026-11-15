# Changelog

Todas as mudanças que importam para quem usa o Scout Vault. O formato segue o
[Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/) e as versões seguem o
[Versionamento Semântico](https://semver.org/lang/pt-BR/). Os downloads ficam nas
[Releases do GitHub](https://github.com/qedrohenrique/scout-vault/releases).

## [Ainda não lançado]

## [0.3.0] - 2026-11-15

### Adicionado
- **Salvar logins pelo navegador:** depois de entrar num site, a extensão pergunta
  "Salvar este login no Scout Vault?" ou "Atualizar a senha de …?". Nada é salvo sem o
  clique; a senha antiga vai para o histórico. Com o cofre bloqueado, o app pede a senha e
  salva em seguida. A extensão passa a rodar em todos os sites (o navegador avisa ao atualizar).
- **Desbloquear com Windows Hello** (rosto, digital ou PIN). A chave do cofre fica cifrada por
  uma assinatura do Windows Hello; trocar a senha mestra ou sair da conta apaga a configuração.
- **Sync em segundo plano:** a cada minuto o app pergunta ao servidor se o cofre mudou e, se
  mudou em outro aparelho, atualiza sozinho.
- **Pedir a senha mestra** em itens marcados (como no Bitwarden): mostrar, copiar, editar e
  digitar esperam a confirmação; a extensão não preenche esses itens.
- **Bloquear junto com o Windows** (Win+L, suspensão e hibernação).
- **Menu da bandeja** com acesso rápido aos itens fixados (copiar usuário, senha ou código),
  "Gerar e copiar senha" e "Sincronizar agora".
- **Itens em organizações:** escolher a organização e as coleções no formulário; coleções na
  barra lateral.
- **Sessão configurável:** abrir sem senha por N dias, pedir login completo a cada N dias e
  tempo de bloqueio por inatividade.
- **Arrastar e soltar:** itens para pastas e pastas para dentro de outras (subpastas).
- `CHANGELOG.md` (este arquivo) e releases automáticas: enviar uma tag `vX.Y.Z` testa,
  compila, empacota e publica a release com as notas daquela versão.

### Mudado
- O menu do botão direito da bandeja é desenhado pelo próprio app, com o tema e a cor de destaque.
- Trocar o tema, a cor de destaque, o texto ou os cantos transforma a interface no lugar: cada
  cor vai da antiga à nova, e entre claro e escuro uma onda sai do botão clicado
  ("Movimento: Parado" troca direto).
- Clicar num item da lista mostra o item mesmo com um formulário aberto.
- Roadmap refeito com um comparativo com Bitwarden, 1Password, KeePassXC e Proton Pass.
- CI no GitHub Actions: formatação, clippy e testes no Windows, testes do núcleo contra um
  Vaultwarden descartável, cargo-deny, CodeQL e Dependabot.

## [0.2.0] - 2026-11-14

### Adicionado
- **Pastas:** criar, renomear e apagar pela barra lateral.
- **Acesso rápido:** itens fixados no topo da barra lateral.
- **Anexos mais fáceis:** clicar ou arrastar arquivos no formulário e no detalhe do item;
  mensagem clara quando a conta não pode enviar anexos (Premium).
- **Bandeja do Windows:** fechar a janela deixa o app rodando; abrir de novo só mostra a janela.
- **Toasts** para avisos e erros.
- **Animações** em telas, barra lateral, seleção, sincronização e carregamento
  (Configurações → Movimento → Parado desliga).

### Mudado
- Formulário mais compacto: campos curtos lado a lado e exemplos dentro dos campos.

## [0.1.0] - 2026-11-14

Primeira versão de teste.

### Adicionado
- Login com e-mail e senha mestra, 2FA (autenticador, e-mail, YubiKey, chave de
  segurança/Windows Hello, Duo) e SSO da empresa; bitwarden.com, bitwarden.eu ou Vaultwarden.
- Sessão lembrada: abre pedindo só a senha mestra, inclusive sem internet.
- Busca, filtros, código do autenticador ao vivo; cópias somem em 30 s e ficam fora do Win+V.
- Criar e editar logins, notas, cartões e identidades; lixeira; histórico de senhas; dados de
  outros apps (passkeys, regras de URI) preservados.
- Anexos: anexar, abrir, salvar e remover.
- Preencher em apps do Windows com `Ctrl+Alt+A`.
- Extensão para Chrome, Edge e Brave que preenche logins.
- Gerador de senhas, temas claro/escuro e cor de destaque.

[Ainda não lançado]: https://github.com/qedrohenrique/scout-vault/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/qedrohenrique/scout-vault/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/qedrohenrique/scout-vault/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/qedrohenrique/scout-vault/releases/tag/v0.1.0
