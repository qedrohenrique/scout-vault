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
  wallet(tab);
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

// A list entry: name and a detail line.
function entry(name, detail, onClick) {
  const li = document.createElement("li");
  const button = document.createElement("button");
  const title = document.createElement("span");
  title.className = "name";
  title.textContent = name;
  button.append(title);
  if (detail) {
    const sub = document.createElement("span");
    sub.className = "user";
    sub.textContent = detail;
    button.append(sub);
  }
  button.addEventListener("click", onClick);
  li.append(button);
  return li;
}

// Cards and identities: the app sends names only; the data comes after a click.
async function fillWallet(tab, kind, id) {
  const data = await ask({ type: kind === "card" ? "fillcard" : "fillidentity", id });
  if (data.error || data.locked) {
    show(explain(data));
    return;
  }
  const [result] = await chrome.scripting.executeScript({
    target: { tabId: tab.id },
    func: scoutVaultFillWallet,
    args: [kind, data],
  });
  if (result && result.result > 0) {
    window.close();
  } else {
    show(tr("popupNoWalletForm"));
  }
}

async function wallet(tab) {
  const response = await ask({ type: "wallet" });
  if (response.error || response.locked) return;
  const cards = response.cards || [];
  const identities = response.identities || [];
  if (!cards.length && !identities.length) return;
  $("#wallet").hidden = false;
  $("#cards-title").hidden = !cards.length;
  $("#identities-title").hidden = !identities.length;
  for (const c of cards) {
    const detail = [c.brand, c.last4 ? `•••• ${c.last4}` : ""].filter(Boolean).join(" ");
    $("#cards").append(entry(c.name, detail, () => fillWallet(tab, "card", c.id)));
  }
  for (const i of identities) {
    $("#identities").append(entry(i.name, i.detail, () => fillWallet(tab, "identity", i.id)));
  }
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
