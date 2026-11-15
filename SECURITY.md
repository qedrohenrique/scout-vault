# Segurança

O Scout Vault guarda e mostra senhas: falhas de segurança são levadas a sério.

## Como relatar uma falha

**Não abra uma issue pública.** Use o relato privado do GitHub:
[Security → Report a vulnerability](https://github.com/qedrohenrique/scout-vault/security/advisories/new).
Só os mantenedores veem o relato.

Ajuda muito incluir:
- a versão (Configurações mostra; ou o nome do zip da release);
- o que dá para fazer com a falha e em que condições (acesso local, outro usuário do Windows,
  extensão maliciosa, servidor comprometido…);
- passos para reproduzir ou uma prova de conceito.

Resposta inicial em até 7 dias. Quando houver correção, sai numa release com crédito a quem
relatou (se quiser) e um aviso de segurança no repositório.

## Versões cobertas

Só a versão mais recente recebe correções. O projeto está em versões de teste (0.x): veja os
avisos nas [releases](https://github.com/qedrohenrique/scout-vault/releases).

## O que está no escopo

- O app (`crates/`): criptografia, guarda local de chaves e sessão, comunicação com o servidor,
  Windows Hello, área de transferência, preenchimento em apps.
- A extensão do navegador (`extension/`) e o canal local entre ela e o app.
- Os workflows de build e release (`.github/`).

Falhas do servidor Bitwarden ou do Vaultwarden vão para os projetos deles:
[Bitwarden](https://bitwarden.com/help/security/) e
[Vaultwarden](https://github.com/dani-garcia/vaultwarden/security).
