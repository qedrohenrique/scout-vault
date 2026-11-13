// Popup da extensão: pede ao app Scout Vault os logins do site aberto e preenche
// o escolhido. Senhas só saem do app quando a pessoa clica num login.
const HOST = "com.scoutvault.desktop";

const $ = (s) => document.querySelector(s);

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
    return 'O Scout Vault não respondeu. Abra o app e ative "Preencher no navegador" nas configurações.';
  }
  if (response.error === "closed") {
    return "O Scout Vault está fechado. Abra o app e tente de novo.";
  }
  if (response.locked) {
    return "O Scout Vault está bloqueado. Desbloqueie no app e tente de novo.";
  }
  return response.error || "Algo deu errado.";
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
    show("Não encontrei o formulário de login nesta página.");
  }
}

async function main() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const url = (tab && tab.url) || "";
  if (!/^https?:\/\//.test(url)) {
    show("Abra um site para preencher.");
    return;
  }
  $("#site").textContent = new URL(url).hostname;
  const response = await ask({ type: "list", url });
  if (response.error || response.locked) {
    show(explain(response));
    return;
  }
  if (!response.items.length) {
    show("Nenhum login salvo para este site.");
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

main();
