// Extension background: receives the login submitted on a site (capture.js), asks the
// app whether it is new or a new password and, after the person's "Save" on the page,
// tells the app to save it. The pending login lives only in the browser's memory
// (chrome.storage.session) and expires in 2 minutes.
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

// What the page needs to show the question (without the password).
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
  // App closed or integration turned off: don't bother the person.
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
  // Page that doesn't reload (SPA): ask right away. If the page changed, the new one asks when it opens.
  chrome.tabs.sendMessage(tabId, promptOf(pending)).catch(() => {});
}

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  const tabId = sender.tab && sender.tab.id;
  if (tabId == null || !msg || typeof msg.type !== "string") return;
  if (msg.type === "captured") {
    // The address is the one of the frame that sent it, as the browser reports it: a page
    // (or an embedded frame) can't make the prompt about another site.
    const from = sender.url || "";
    if (/^https?:\/\//.test(from)) captured(tabId, { ...msg, url: from });
    return;
  }
  if (msg.type === "pending") {
    getPending(tabId).then((p) => sendResponse(p ? promptOf(p) : null));
    return true;
  }
  if (msg.type === "confirm") {
    getPending(tabId).then(async (p) => {
      if (!p) {
        sendResponse({ error: chrome.i18n.getMessage("errExpired") });
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
    return;
  }
  // Inline menu: the address is ALWAYS the one of the frame that asked (sender.url), never
  // one that comes in the message — a page can't ask for another site's logins.
  const frameUrl = sender.url || "";
  if (!/^https?:\/\//.test(frameUrl)) return;
  if (msg.type === "inline-list") {
    ask({ type: "list", url: frameUrl }).then(sendResponse);
    return true;
  }
  if (msg.type === "inline-fill" && typeof msg.id === "string") {
    ask({ type: "fill", id: msg.id, url: frameUrl }).then(sendResponse);
    return true;
  }
});

// "Open the menu in the login field" shortcut (Alt+Shift+L by default).
chrome.commands.onCommand.addListener(async (command) => {
  if (command !== "inline-menu") return;
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (tab && tab.id != null) {
    chrome.tabs.sendMessage(tab.id, { type: "open-inline" }, { frameId: 0 }).catch(() => {});
  }
});

chrome.tabs.onRemoved.addListener((tabId) => clearPending(tabId));
