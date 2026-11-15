// Fundo da extensão: recebe o login enviado num site (capture.js), pergunta ao
// app se é novo ou uma senha nova e, depois do "Salvar" da pessoa na página,
// manda o app salvar. O login pendente fica só na memória do navegador
// (chrome.storage.session) e expira em 2 minutos.
const HOST = "com.scoutvault.desktop";
const TTL_MS = 2 * 60 * 1000;

function ask(message) {
  return new Promise((resolve) => {
    chrome.runtime.sendNativeMessage(HOST, message, (response) => {
      if (chrome.runtime.lastError || !response) {
        resolve({ error: "app" });
      } else {
        resolve(response);
      }
    });
  });
}

const key = (tabId) => `pendente-${tabId}`;

async function getPending(tabId) {
  const k = key(tabId);
  const stored = (await chrome.storage.session.get(k))[k];
  if (!stored) return null;
  if (Date.now() - stored.at > TTL_MS) {
    await chrome.storage.session.remove(k);
    return null;
  }
  return stored;
}

const clearPending = (tabId) => chrome.storage.session.remove(key(tabId));

// O que a página precisa para mostrar a pergunta (sem a senha).
const promptOf = (p) => ({
  type: "prompt",
  action: p.action,
  name: p.name,
  host: p.host,
  username: p.username,
});

async function captured(tabId, msg) {
  const answer = await ask({
    type: "check",
    url: msg.url,
    username: msg.username,
    password: msg.password,
  });
  // App fechado ou integração desligada: não incomoda.
  if (answer.error) return;
  const action = answer.locked ? "save-locked" : answer.action;
  if (action !== "save" && action !== "update" && action !== "save-locked") return;
  const pending = {
    action,
    name: answer.name || "",
    host: new URL(msg.url).hostname,
    url: msg.url,
    username: msg.username,
    password: msg.password,
    at: Date.now(),
  };
  await chrome.storage.session.set({ [key(tabId)]: pending });
  // Página que não recarrega (SPA): pergunta já. Se a página mudou, a nova pergunta ao abrir.
  chrome.tabs.sendMessage(tabId, promptOf(pending)).catch(() => {});
}

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  const tabId = sender.tab && sender.tab.id;
  if (tabId == null || !msg || typeof msg.type !== "string") return;
  if (msg.type === "captured") {
    captured(tabId, msg);
    return;
  }
  if (msg.type === "pending") {
    getPending(tabId).then((p) => sendResponse(p ? promptOf(p) : null));
    return true;
  }
  if (msg.type === "confirm") {
    getPending(tabId).then(async (p) => {
      if (!p) {
        sendResponse({ error: "A pergunta expirou. Envie o login de novo para salvar." });
        return;
      }
      await clearPending(tabId);
      sendResponse(
        await ask({ type: "save", url: p.url, username: p.username, password: p.password }),
      );
    });
    return true;
  }
  if (msg.type === "dismiss") {
    clearPending(tabId);
  }
});

chrome.tabs.onRemoved.addListener((tabId) => clearPending(tabId));
