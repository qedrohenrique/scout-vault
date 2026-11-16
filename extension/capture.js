// Runs on every site: notices when a login (username + password) is submitted
// and, if the app says it is new or a new password, shows a discreet bar
// "Save in Scout Vault?". Nothing is saved without the person's click.
(() => {
  if (window.__scoutVaultCapture) return;
  window.__scoutVaultCapture = true;
  if (window.top !== window) return; // main page only
  const tr = (key, ...subs) => chrome.i18n.getMessage(key, subs) || key;

  const describe = (i) =>
    `${i.name || ""} ${i.id || ""} ${i.getAttribute("autocomplete") || ""} ${i.placeholder || ""}`;
  const isUserLike = (i) =>
    ["text", "email", "tel", ""].includes(i.type) &&
    !/search|captcha|otp|one-time|\bcode\b|cupom|coupon/i.test(describe(i));
  const auto = (i) => (i.getAttribute("autocomplete") || "").toLowerCase();

  // Username and password filled in the form (or in the page, if there is no <form>).
  function grab(root) {
    const scope = [...(root || document).querySelectorAll("input")];
    const passwords = scope.filter((i) => i.type === "password" && i.value);
    if (!passwords.length) return null;
    // Password change (current + new + confirmation): keep the new one.
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
    if (signature === lastSent) return; // submit + click in the same submission
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

  // ---- "Save in Scout Vault?" bar ------------------------------------------

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
      ? tr("captureUpdateTitle", p.name || "")
      : tr("captureSaveTitle");
    const detail =
      p.action === "save-locked"
        ? tr("captureLockedDetail")
        : `${p.username || tr("captureNoUser")} · ${p.host}`;
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
          <button class="ghost no"></button>
          <button class="primary yes"></button>
        </div>
      </div>`;
    root.querySelector(".title").textContent = title;
    root.querySelector(".detail").textContent = detail;
    root.querySelector(".no").textContent = tr("captureNotNow");
    const yes = root.querySelector(".yes");
    yes.textContent = update ? tr("captureUpdate") : tr("captureSave");
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
        say(r && r.error && r.error !== "app" ? r.error : tr("noAnswer"));
      } else if (r.queued) {
        say(tr("captureUnlockToFinish"));
      } else {
        say(update ? tr("captureUpdated") : tr("captureSaved"));
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
  // New page after the submission: is there a question waiting?
  chrome.runtime
    .sendMessage({ type: "pending" })
    .then((p) => p && showBar(p))
    .catch(() => {});
})();
