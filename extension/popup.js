// Extension popup: asks the Scout Vault app for the open site's logins and fills
// the chosen one. Passwords only leave the app when the person clicks a login.
const HOST = "com.scoutvault.desktop";

const $ = (s) => document.querySelector(s);
const tr = (key, ...subs) => chrome.i18n.getMessage(key, subs) || key;

// Fixed HTML texts: data-i18n attribute with the message key.
document.documentElement.lang = chrome.i18n.getUILanguage();
for (const el of document.querySelectorAll("[data-i18n]")) el.textContent = tr(el.dataset.i18n);

function show(text) {
  $("#message").textContent = text;
  $("#items").replaceChildren();
}

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

function explain(response) {
  if (response.error === "app") {
    return tr("errAppSetup");
  }
  if (response.error === "closed") {
    return tr("errClosed");
  }
  if (response.locked) {
    return tr("errLocked");
  }
  return response.error || tr("errGeneric");
}

async function fill(tab, url, id) {
  const creds = await ask({ type: "fill", id, url });
  if (creds.error || creds.locked) {
    show(explain(creds));
    return;
  }
  const [result] = await chrome.scripting.executeScript({
    target: { tabId: tab.id },
    func: scoutVaultFillPage,
    args: [creds.username || "", creds.password || ""],
  });
  const filled = result && result.result;
  if (filled && (filled.username || filled.password)) {
    window.close();
  } else {
    show(tr("popupNoForm"));
  }
}

async function main() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const url = (tab && tab.url) || "";
  if (!/^https?:\/\//.test(url)) {
    show(tr("popupOpenSite"));
    return;
  }
  $("#site").textContent = new URL(url).hostname;
  const response = await ask({ type: "list", url });
  if (response.error || response.locked) {
    show(explain(response));
    return;
  }
  if (!response.items.length) {
    show(tr("popupNoLogins"));
    return;
  }
  $("#message").textContent = "";
  const list = $("#items");
  for (const item of response.items) {
    const li = document.createElement("li");
    const button = document.createElement("button");
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = item.name;
    button.append(name);
    if (item.username) {
      const user = document.createElement("span");
      user.className = "user";
      user.textContent = item.username;
      button.append(user);
    }
    button.addEventListener("click", () => fill(tab, url, item.id));
    li.append(button);
    list.append(li);
  }
  list.querySelector("button").focus();
}

// Inline menu switch (inline.js): on by default.
async function setupToggle() {
  const box = $("#inline-menu");
  const { inlineMenu } = await chrome.storage.local.get({ inlineMenu: true });
  box.checked = inlineMenu !== false;
  box.addEventListener("change", () => chrome.storage.local.set({ inlineMenu: box.checked }));

  // Site hidden by the menu's "Don't show on this site": can be undone here.
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab || !/^https?:\/\//.test(tab.url || "")) return;
  const host = new URL(tab.url).hostname;
  const { inlineOff } = await chrome.storage.local.get({ inlineOff: [] });
  if (!inlineOff.includes(host)) return;
  const again = $("#inline-site");
  again.hidden = false;
  again.addEventListener("click", async () => {
    const now = (await chrome.storage.local.get({ inlineOff: [] })).inlineOff;
    await chrome.storage.local.set({ inlineOff: now.filter((h) => h !== host) });
    again.hidden = true;
  });
}

setupToggle();
main();
