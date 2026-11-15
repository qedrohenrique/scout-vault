# Roteiro de teste com conta real (bitwarden.com / bitwarden.eu)

O Scout Vault só **lê** o cofre: ele não cria, não edita e não apaga nada no servidor. Mesmo assim,
comece com uma **conta de teste**. Use a conta real só depois que a de teste passar.

## Antes de começar
1. Crie uma conta gratuita em https://vault.bitwarden.com com um e-mail que você consiga abrir.
2. No cofre web, cadastre alguns itens: um login com usuário, senha e site; um cartão; uma nota;
   uma pasta; e um item marcado como favorito.
3. Rode o app com `cargo run`. Na tela de login, escolha **bitwarden.com** (ou **bitwarden.eu**,
   se a conta for europeia).

## Casos para testar
Faça os casos em ordem. Anote o texto exato que aparecer embaixo do formulário.

| # | Cenário | O que fazer | Esperado |
|---|---|---|---|
| 1 | Senha errada | Entre com a senha errada | "E-mail ou senha mestra incorretos." |
| 2 | Primeiro login, sem 2FA | Entre com a senha certa | O servidor pode pedir o **código de novo dispositivo** por e-mail. Digite-o e o cofre abre |
| 3 | Código de dispositivo errado | No passo do item 2, digite um código errado | Aparece um erro e a tela **continua no mesmo passo**; dá para digitar o código certo |
| 4 | Itens | Abra cada item, revele e copie a senha | Os valores batem com o cofre web; a área de transferência é limpa em 30 s |
| 5 | Sincronizar | Mude um item no cofre web e clique em sincronizar | A mudança aparece |
| 6 | Bloquear e desbloquear | Bloqueie e desbloqueie com a senha | Abre sem rede |
| 7 | 2FA por autenticador | Ative o autenticador no cofre web, saia e entre de novo | O app pede o código; um código errado mantém a tela e o certo abre o cofre |
| 8 | 2FA por e-mail | Ative o 2FA por e-mail e clique em "Enviar código por e-mail" | "Código enviado." e o e-mail chega |
| 9 | Lembrar dispositivo | Marque "Lembrar este dispositivo", entre, feche o app e entre de novo | O segundo login **não pede código**. O token fica no Gerenciador de Credenciais do Windows, como "Scout Vault" |
| 10 | Chave de segurança / Windows Hello | Cadastre uma chave FIDO2 (ou o Windows Hello) no cofre web como 2FA e escolha esse método no app | O botão "Usar chave de segurança ou Windows Hello" abre o pedido do Windows; ao tocar na chave, o cofre abre |
| 11 | Duo | Com Duo ativo, clique em "Abrir o Duo no navegador" e aprove. O navegador tenta abrir `bitwarden://`; cancele, copie o endereço da página e cole no campo | O cofre abre |
| 12 | SSO (só com organização) | Marque "Entrar com o SSO da empresa", informe o identificador SSO e a senha mestra | O navegador abre o login da empresa; ao concluir, a página diz para voltar e o cofre abre |
| 13 | Preencher em outro app | Com o cofre aberto, abra a tela de login de um app ou site cujo **título da janela** tenha o nome do item ou o site (ex.: "Steam", "github.com"), clique no campo de usuário e aperte `Ctrl+Alt+A` | O Scout Vault digita usuário ⇥ senha ↵. Se mais de um item combinar (ou nenhum), o Scout Vault aparece com a lista; ao escolher, volta para a janela e digita |
| 14 | Preencher com o cofre bloqueado | Bloqueie o cofre e aperte `Ctrl+Alt+A` no app | O Scout Vault aparece pedindo a senha; depois de desbloquear, digita no app |
| 15 | Lembrar sessão | Entre, feche o app e abra de novo | Abre direto no **desbloqueio** (só a senha). Sem internet, desbloqueia e mostra o cofre |
| 16 | Criar e editar | Crie um login pelo `+`, depois edite a senha | Aparece no cofre web e no celular; no web vault, o **histórico de senhas** do item mostra a antiga |
| 17 | Dados de outros apps | Edite no Scout Vault um item que tenha **passkey** ou regra de URI criada no app oficial | A passkey e a regra continuam lá no app oficial |
| 18 | Lixeira | Mova para a lixeira, restaure, mova de novo e apague de vez | Bate com o cofre web a cada passo |
| 19 | Anexos | Crie uma **Nota** e, no próprio formulário, arraste um PDF e uma imagem para "Anexos"; salve. Depois arraste mais um arquivo no detalhe do item | Os arquivos aparecem no cofre web e abrem iguais. Em conta gratuita do bitwarden.com, o app avisa que anexos exigem Premium. O envio para a nuvem oficial (Azure) **ainda não foi testado** |
| 20 | Navegador | Configurações → Navegador → ligar; carregue a extensão; num site salvo, clique no ícone do Scout Vault | Lista os logins do site; clicar preenche usuário e senha |
| 21 | Pastas | Crie uma pasta pelo `+` da barra lateral, ponha um item nela, renomeie e apague | A pasta aparece igual no cofre web; ao apagar, o item continua lá, em "Sem pasta" |
| 22 | Organização | Numa organização, abra uma coleção na barra lateral, crie um login pelo `+` e salve; depois edite e marque outra coleção | O item aparece na organização e nas coleções certas no cofre web; quem só tem a outra coleção passa a vê-lo |
| 23 | Ficar conectado | Em Configurações → Sessão, ponha "Pedir a senha mestra ao abrir" em 7 dias; desbloqueie, saia pela bandeja e abra de novo. Depois troque a senha mestra no cofre web e sincronize | Reabre já desbloqueado; depois da troca de senha aparece o aviso e a próxima abertura pede a senha nova |
| 24 | Windows Hello | Com o cofre aberto, em Configurações → Sessão ligue "Desbloquear com Windows Hello" e confirme o rosto/PIN. Bloqueie (`Ctrl+L`) e use "Usar Windows Hello". Depois cancele o Hello uma vez | O primeiro aviso do Windows Hello aparece **na frente** do app; o botão abre o cofre; cancelar explica e deixa a senha como alternativa. **Ainda não testado com o Windows Hello de verdade** |
| 25 | Bloquear com o Windows e re-prompt | Com o cofre aberto, aperte Win+L e entre de novo no Windows. Depois marque "Pedir a senha mestra" num item e tente mostrar/copiar a senha | O cofre está bloqueado ao voltar. No item marcado, a senha só aparece depois de confirmar a senha mestra; o app oficial também passa a pedir |
| 26 | Salvar logins | Recarregue a extensão (chrome://extensions → ↻; ela pede permissão para todos os sites). Entre num site com um login que não está no cofre; depois troque a senha desse site e entre de novo | Aparece "Salvar este login no Scout Vault?" e o item surge no cofre (e no web vault). Na segunda vez, "Atualizar a senha de …?"; o web vault mostra a antiga no histórico. "Agora não" não salva nada |

## Se algo falhar
Para erros do servidor, o app mostra o código HTTP e a mensagem que o servidor mandou. Copie esse
texto e me passe junto com:
- o número do caso;
- se a conta é US ou EU;
- quais métodos de 2FA estão ativos na conta.

**Não mande senhas, códigos nem o conteúdo de `settings.json`.** O identificador de dispositivo
que fica nesse arquivo não é secreto, mas também não é necessário.

## Limitações conhecidas
- **SSO:** o cofre abre com a **senha mestra**, como no CLI oficial. Contas que dependem de
  **Key Connector** ou de **dispositivo confiável** (sem senha mestra) não abrem; o app explica.
  Se a empresa exigir 2FA depois do SSO, o app avisa que esse caminho ainda não é suportado.
- **Chave de segurança:** só no Windows (usa a API nativa `webauthn.dll`).
- **Duo:** exige colar o endereço final, porque o servidor só redireciona para o app oficial
  (`bitwarden://`). Se o app oficial estiver instalado, o navegador vai oferecer abri-lo: recuse.
- **Captcha:** se o servidor pedir, o app não consegue mostrá-lo. Entre uma vez pelo cofre web e
  tente de novo.
- **Tipos de item novos** (conta bancária, CNH, passaporte) aparecem como "Tipo N ainda não
  suportado".

## O que mudou para o servidor oficial
- O pedido de código por e-mail envia o `ssoEmail2faSessionToken` que o servidor oficial devolve
  junto com o pedido de 2FA. Ele também continua mandando o hash da senha, que o Vaultwarden usa.
- O login envia o cabeçalho `Device-Identifier` e o campo `devicePushToken`.
- O app agora reconhece estas respostas: código de dispositivo inválido (`device_error`), código de
  2FA inválido (que antes aparecia como "senha incorreta") e captcha.
- Se o sync não trouxer a user key, o app usa a chave que veio na resposta do login.
- Erros desconhecidos mostram a mensagem do servidor, para facilitar o diagnóstico.
- **Criptografia v2** (EncString tipo 7, COSE): decifra XChaCha20-Poly1305, XAES-256-GCM,
  AES-256-GCM e o AEAD AES-CBC-HMAC; validada com os vetores de teste públicos do Bitwarden.
- **Lembrar dispositivo**, **chave de segurança**, **Duo** e **SSO**: veja os casos 9 a 12.

## O que foi testado aqui
- Contra o Vaultwarden local: login, sync, 2FA por autenticador com código errado e certo,
  "lembrar dispositivo" entre logins e token expirado (`tests/live_2fa.rs`).
- **Sem servidor real disponível:** SSO, chave de segurança e Duo têm testes das partes locais
  (PKCE, retorno do navegador, formato do token), mas o caminho completo depende do seu teste.
