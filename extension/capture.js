// Roda em todos os sites: percebe quando um login (usuário + senha) é enviado
// e, se o app disser que é novo ou uma senha nova, mostra uma barra discreta
// "Salvar no Scout Vault?". Nada é salvo sem o clique da pessoa.
(() => {
  if (window.__scoutVaultCapture) return;
  window.__scoutVaultCapture = true;
  if (window.top !== window) return; // só a página principal

  const describe = (i) =>
    `${i.name || ""} ${i.id || ""} ${i.getAttribute("autocomplete") || ""} ${i.placeholder || ""}`;
  const isUserLike = (i) =>
    ["text", "email", "tel", ""].includes(i.type) &&
    !/search|captcha|otp|one-time|\bcode\b|cupom|coupon/i.test(describe(i));
  const auto = (i) => (i.getAttribute("autocomplete") || "").toLowerCase();

  // Usuário e senha preenchidos no formulário (ou na página, se não houver <form>).
  function grab(root) {
    const scope = [...(root || document).querySelectorAll("input")];
    const passwords = scope.filter((i) => i.type === "password" && i.value);
    if (!passwords.length) return null;
    // Troca de senha (atual + nova + confirmação): fica a nova.
    const password = passwords[passwords.length - 1].value;
    const first = scope.indexOf(passwords[0]);
    const before = scope.slice(0, first).filter((i) => isUserLike(i) && i.value.trim());
    const user =
      scope.find((i) => /username|email/.test(auto(i)) && isUserLike(i) && i.value.trim()) ||
      before[before.length - 1];
    return { username: user ? user.value.trim() : "", password };
  }

  let lastSent = "";
  function capture(root) {
    const found = grab(root);
    if (!found) return;
    const signature = `${location.host}|${found.username}|${found.password.length}`;
    if (signature === lastSent) return; // submit + clique no mesmo envio
    lastSent = signature;
    setTimeout(() => (lastSent = ""), 1500);
    chrome.runtime.sendMessage({ type: "captured", url: location.href, ...found }).catch(() => {});
  }

  document.addEventListener("submit", (e) => capture(e.target), true);
  document.addEventListener(
    "click",
    (e) => {
      const button = e.target.closest(
        'button, input[type="submit"], input[type="button"], [role="button"]',
      );
      if (!button) return;
      capture(button.form || button.closest("form"));
    },
    true,
  );
  document.addEventListener(
    "keydown",
    (e) => {
      if (e.key === "Enter" && e.target instanceof HTMLInputElement) {
        capture(e.target.form);
      }
    },
    true,
  );

  // ---- Barra "Salvar no Scout Vault?" -------------------------------------

  let host = null;
  function closeBar() {
    if (host) host.remove();
    host = null;
  }

  function showBar(p) {
    closeBar();
    host = document.createElement("div");
    host.style.cssText = "all: initial; position: fixed; top: 16px; right: 16px; z-index: 2147483647;";
    const root = host.attachShadow({ mode: "closed" });
    const update = p.action === "update";
    const title = update
      ? `Atualizar a senha de “${p.name}” no Scout Vault?`
      : "Salvar este login no Scout Vault?";
    const detail =
      p.action === "save-locked"
        ? "O app vai pedir para desbloquear o cofre."
        : `${p.username || "(sem usuário)"} · ${p.host}`;
    root.innerHTML = `
      <style>
        .bar { font: 13px/1.4 system-ui, "Segoe UI", sans-serif; width: 320px; padding: 14px 16px;
               border-radius: 12px; background: #fff; color: #1d1d1f; border: 1px solid #d0d4dc;
               box-shadow: 0 10px 30px rgba(0,0,0,.18); animation: in .22s ease-out; }
        @media (prefers-color-scheme: dark) {
          .bar { background: #1f2126; color: #eceef2; border-color: #3a3d45; }
          .muted { color: #a3a7b0 !important; }
          .ghost { color: #eceef2 !important; }
        }
        @keyframes in { from { opacity: 0; transform: translateY(-8px); } to { opacity: 1; transform: none; } }
        .head { display: flex; gap: 8px; align-items: center; font-weight: 600; }
        .dot { width: 18px; height: 18px; border-radius: 5px; background: #4f6bed; flex: none; }
        .muted { color: #5f6470; margin: 4px 0 12px 26px; word-break: break-all; }
        .row { display: flex; gap: 8px; justify-content: flex-end; }
        button { font: inherit; border-radius: 8px; padding: 6px 12px; cursor: pointer; border: 0; }
        .primary { background: #4f6bed; color: #fff; }
        .ghost { background: transparent; color: #1d1d1f; }
        button:disabled { opacity: .6; cursor: default; }
      </style>
      <div class="bar" role="dialog" aria-label="Scout Vault">
        <div class="head"><span class="dot"></span><span class="title"></span></div>
        <div class="muted detail"></div>
        <div class="row">
          <button class="ghost no">Agora não</button>
          <button class="primary yes"></button>
        </div>
      </div>`;
    root.querySelector(".title").textContent = title;
    root.querySelector(".detail").textContent = detail;
    const yes = root.querySelector(".yes");
    yes.textContent = update ? "Atualizar" : "Salvar";
    root.querySelector(".no").addEventListener("click", () => {
      chrome.runtime.sendMessage({ type: "dismiss" }).catch(() => {});
      closeBar();
    });
    yes.addEventListener("click", async () => {
      yes.disabled = true;
      const r = await chrome.runtime.sendMessage({ type: "confirm" }).catch(() => null);
      const say = (text) => {
        root.querySelector(".title").textContent = text;
        root.querySelector(".detail").textContent = "";
        root.querySelector(".row").remove();
        setTimeout(closeBar, 2600);
      };
      if (!r || r.error) {
        say(r && r.error && r.error !== "app" ? r.error : "O Scout Vault não respondeu.");
      } else if (r.queued) {
        say("Desbloqueie o Scout Vault para concluir.");
      } else {
        say(update ? "Senha atualizada no Scout Vault ✓" : "Salvo no Scout Vault ✓");
      }
    });
    (document.body || document.documentElement).append(host);
    setTimeout(() => {
      if (host && !yes.disabled) closeBar();
    }, 30000);
  }

  chrome.runtime.onMessage.addListener((msg) => {
    if (msg && msg.type === "prompt") showBar(msg);
  });
  // Página nova depois do envio: há uma pergunta esperando?
  chrome.runtime
    .sendMessage({ type: "pending" })
    .then((p) => p && showBar(p))
    .catch(() => {});
})();
