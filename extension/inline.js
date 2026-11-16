// Inline menu: when a login field gets focus, shows the Scout Vault icon on the
// field's edge; the click (or Alt+↓) opens the list of the site's logins and the
// chosen one is filled right there. Only names and usernames reach the page before the
// click; the password comes from the app at fill time and isn't kept.
(() => {
  if (window.__scoutVaultInline) return;
  window.__scoutVaultInline = true;
  if (window.top !== window) return; // main page only
  const tr = (key, ...subs) => chrome.i18n.getMessage(key, subs) || key;

  const CACHE_MS = 15 * 1000;
  const CLICK_DELAY_MS = 300; // anti-clickjacking: menu visible for at least this long
  const ICON = 22;
  const LOCK_SVG =
    '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round">' +
    '<rect x="3.5" y="7" width="9" height="7" rx="1.5"/><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2"/></svg>';

  // ---- Which fields are login fields ---------------------------------------

  const describe = (i) =>
    `${i.name || ""} ${i.id || ""} ${i.getAttribute("autocomplete") || ""} ${i.placeholder || ""}`;
  const auto = (i) => (i.getAttribute("autocomplete") || "").toLowerCase();
  const isUserLike = (i) =>
    ["text", "email", "tel", ""].includes(i.type) &&
    !/search|captcha|otp|one-time|\bcode\b|cupom|coupon/i.test(describe(i));

  function isLoginField(el) {
    if (!(el instanceof HTMLInputElement) || el.disabled || el.readOnly) return false;
    if (/one-time-code/.test(auto(el))) return false;
    if (el.type === "password") return true;
    if (!isUserLike(el)) return false;
    if (/\b(username|email)\b/.test(auto(el))) return true;
    const scope = el.form || document;
    if (scope.querySelector('input[type="password"]')) return true;
    return /user|email|login|cpf/i.test(describe(el));
  }

  const visible = (el) => {
    if (!el.isConnected) return false;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none";
  };

  // ---- Talking to the background -------------------------------------------

  // Only names and usernames; one request at a time, reused for a few seconds.
  let cached = null; // { at, response: Promise }
  function listLogins(fresh) {
    if (!fresh && cached && Date.now() - cached.at < CACHE_MS) return cached.response;
    const response = chrome.runtime
      .sendMessage({ type: "inline-list" })
      .catch(() => null)
      .then((r) => r || { error: "app" });
    cached = { at: Date.now(), response };
    return response;
  }
  const appMissing = (r) => !r || r.error === "app" || r.error === "closed";

  async function settings() {
    const s = await chrome.storage.local
      .get({ inlineMenu: true, inlineOff: [] })
      .catch(() => ({ inlineMenu: true, inlineOff: [] }));
    return { on: s.inlineMenu !== false && !s.inlineOff.includes(location.hostname) };
  }

  // ---- Interface (closed shadow DOM) ---------------------------------------

  const host = document.createElement("div");
  host.style.cssText =
    "all: initial; position: fixed; left: 0; top: 0; width: 0; height: 0; z-index: 2147483647;";
  const root = host.attachShadow({ mode: "closed" });
  root.innerHTML = `
    <style>
      :host { all: initial; }
      * { box-sizing: border-box; }
      .ui { --bg: #fbfaf7; --fg: #1f1e1b; --muted: #6f6b63; --line: #e2ded5;
            --hover: #f0ece3; --accent: #1f9e77; --accent-fg: #fff;
            font: 13px/1.35 "Segoe UI", system-ui, sans-serif; color: var(--fg); }
      @media (prefers-color-scheme: dark) {
        .ui { --bg: #1b1b1e; --fg: #ecebe8; --muted: #9a978f; --line: #2f2f34;
              --hover: #26262b; --accent: #38cd9e; --accent-fg: #0f1a15; }
      }
      .icon { position: fixed; width: ${ICON}px; height: ${ICON}px; border-radius: 6px;
              display: none; align-items: center; justify-content: center; cursor: pointer;
              background: var(--accent); color: var(--accent-fg);
              box-shadow: 0 1px 3px rgba(0,0,0,.25); transition: transform .12s ease; }
      .icon:hover { transform: scale(1.08); }
      .icon.show { display: flex; animation: pop .16s ease-out; }
      .icon svg { width: 14px; height: 14px; }
      .badge { position: absolute; right: -5px; top: -5px; min-width: 15px; height: 15px;
               padding: 0 4px; border-radius: 8px; font: 600 10px/15px "Segoe UI", system-ui, sans-serif;
               text-align: center; background: var(--fg); color: var(--bg); }
      .badge:empty { display: none; }
      .badge.locked { padding: 0; width: 15px; display: flex; align-items: center;
                      justify-content: center; background: #e0a526; color: #1f1e1b; }
      .badge svg { width: 9px; height: 9px; }
      .menu { position: fixed; display: none; flex-direction: column; overflow: hidden;
              min-width: 260px; max-width: 360px; border-radius: 12px;
              background: var(--bg); border: 1px solid var(--line);
              box-shadow: 0 12px 32px rgba(0,0,0,.22); }
      .menu.show { display: flex; animation: drop .16s ease-out; }
      @keyframes pop { from { opacity: 0; transform: scale(.7); } to { opacity: 1; transform: none; } }
      @keyframes drop { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: none; } }
      .head { display: flex; align-items: center; gap: 8px; padding: 9px 12px;
              border-bottom: 1px solid var(--line); font-weight: 600; }
      .mark { width: 16px; height: 16px; border-radius: 5px; background: var(--accent);
              color: var(--accent-fg); display: flex; align-items: center; justify-content: center; }
      .mark svg { width: 10px; height: 10px; }
      .site { margin-left: auto; color: var(--muted); font-weight: 400; font-size: 12px;
              overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 160px; }
      .list { max-height: 240px; overflow-y: auto; padding: 4px; }
      .item { display: flex; align-items: center; gap: 10px; width: 100%; padding: 7px 8px;
              border-radius: 8px; cursor: pointer; }
      .item.active, .item:hover { background: var(--hover); }
      .avatar { width: 28px; height: 28px; border-radius: 8px; flex: none; display: flex;
                align-items: center; justify-content: center; font-weight: 600;
                background: color-mix(in srgb, var(--accent) 16%, transparent); color: var(--accent); }
      .text { min-width: 0; display: flex; flex-direction: column; }
      .name, .user { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .user { color: var(--muted); font-size: 12px; }
      .note { padding: 12px; color: var(--muted); }
      .note strong { display: block; color: var(--fg); margin-bottom: 2px; }
      .retry { margin-top: 8px; font: inherit; border: 0; border-radius: 7px; padding: 5px 10px;
               background: var(--accent); color: var(--accent-fg); cursor: pointer; }
      .foot { border-top: 1px solid var(--line); padding: 6px 12px; font-size: 12px;
              color: var(--muted); cursor: pointer; }
      .foot:hover { color: var(--fg); }
      .keys { float: right; opacity: .8; }
    </style>
    <div class="ui">
      <div class="icon" role="button">
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8"
             stroke-linecap="round" stroke-linejoin="round"><circle cx="5.5" cy="10.5" r="3"/>
             <path d="M7.7 8.3 13.5 2.5M11.5 4.5l2 2M10 6l1.5 1.5"/></svg>
        <span class="badge"></span>
      </div>
      <div class="menu" role="listbox">
        <div class="head"><span class="mark"><svg viewBox="0 0 16 16" fill="none" stroke="currentColor"
             stroke-width="2.4" stroke-linecap="round"><circle cx="5.5" cy="10.5" r="3"/>
             <path d="M7.7 8.3 13.5 2.5"/></svg></span>Scout Vault<span class="site"></span></div>
        <div class="list"></div>
        <div class="foot" role="button"><span class="hide-site"></span><span class="keys">↑↓ Enter · Esc</span></div>
      </div>
    </div>`;
  const $ = (s) => root.querySelector(s);
  const icon = $(".icon");
  const badge = $(".badge");
  const menu = $(".menu");
  const list = $(".list");
  icon.setAttribute("aria-label", tr("inlineOpen"));
  menu.setAttribute("aria-label", tr("inlineMenuLabel"));
  $(".hide-site").textContent = tr("inlineHide");

  let field = null; // current login field
  let shownAt = 0; // when the menu/icon became visible
  let entries = []; // logins shown
  let active = -1;
  let busy = false;

  function attach() {
    if (!host.isConnected) document.documentElement.append(host);
  }

  function place() {
    if (!field) return;
    if (!visible(field)) {
      hideAll();
      return;
    }
    const r = field.getBoundingClientRect();
    const small = r.width < 80 || r.height < 18;
    icon.style.display = small ? "none" : "";
    icon.style.left = `${Math.round(r.right - ICON - Math.min(8, (r.height - ICON) / 2 + 2))}px`;
    icon.style.top = `${Math.round(r.top + (r.height - ICON) / 2)}px`;
    if (menu.classList.contains("show")) {
      const width = Math.min(360, Math.max(260, r.width));
      const left = Math.max(8, Math.min(r.left, innerWidth - width - 8));
      const below = innerHeight - r.bottom;
      const height = menu.offsetHeight || 200;
      menu.style.width = `${width}px`;
      menu.style.left = `${Math.round(left)}px`;
      menu.style.top =
        below < height + 12 && r.top > height + 12
          ? `${Math.round(r.top - height - 6)}px`
          : `${Math.round(r.bottom + 6)}px`;
    }
  }

  let frame = 0;
  const schedule = () => {
    if (!field || frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      place();
    });
  };
  addEventListener("scroll", schedule, true);
  addEventListener("resize", schedule);

  function closeMenu() {
    menu.classList.remove("show");
    entries = [];
    active = -1;
  }

  function hideAll() {
    closeMenu();
    icon.classList.remove("show");
    field = null;
  }

  async function showIcon(el) {
    const { on } = await settings();
    if (!on || document.activeElement !== el) return;
    const r = await listLogins(false);
    if (appMissing(r) || document.activeElement !== el) return;
    if (field !== el) closeMenu();
    field = el;
    attach();
    if (r.locked) {
      badge.innerHTML = LOCK_SVG; // our own constant, no page data
    } else {
      badge.textContent = r.items && r.items.length ? String(r.items.length) : "";
    }
    badge.classList.toggle("locked", !!r.locked);
    if (!icon.classList.contains("show")) shownAt = Date.now();
    icon.classList.add("show");
    place();
  }

  const initial = (name) => (name || "?").trim().charAt(0).toUpperCase() || "?";

  function note(title, detail, retry) {
    const box = document.createElement("div");
    box.className = "note";
    const strong = document.createElement("strong");
    strong.textContent = title;
    box.append(strong, document.createTextNode(detail));
    if (retry) {
      const b = document.createElement("button");
      b.className = "retry";
      b.textContent = tr("inlineRetry");
      b.addEventListener("click", (e) => {
        if (trusted(e)) openMenu(true);
      });
      box.append(document.createElement("br"), b);
    }
    list.replaceChildren(box);
  }

  function setActive(i) {
    const rows = [...list.querySelectorAll(".item")];
    if (!rows.length) return;
    active = (i + rows.length) % rows.length;
    rows.forEach((row, n) => {
      row.classList.toggle("active", n === active);
      row.setAttribute("aria-selected", String(n === active));
    });
    rows[active].scrollIntoView({ block: "nearest" });
  }

  async function openMenu(fresh) {
    if (!field) return;
    const el = field;
    $(".site").textContent = location.hostname.replace(/^www\./, "");
    list.replaceChildren();
    note(tr("searching"), "");
    if (!menu.classList.contains("show")) shownAt = Date.now();
    menu.classList.add("show");
    place();
    const r = await listLogins(fresh);
    if (field !== el) return;
    if (appMissing(r)) {
      note(tr("noAnswer"), tr("inlineAppHint"), true);
    } else if (r.locked) {
      note(tr("vaultLocked"), tr("inlineUnlockRetry"), true);
    } else if (r.error) {
      note(tr("errGeneric"), r.error, true);
    } else if (!r.items.length) {
      note(tr("inlineNoLogins"), tr("inlineForSite", location.hostname.replace(/^www\./, "")), false);
    } else {
      entries = r.items;
      list.replaceChildren(
        ...entries.map((item, n) => {
          const row = document.createElement("div");
          row.className = "item";
          row.setAttribute("role", "option");
          const avatar = document.createElement("span");
          avatar.className = "avatar";
          avatar.textContent = initial(item.name);
          const text = document.createElement("span");
          text.className = "text";
          const name = document.createElement("span");
          name.className = "name";
          name.textContent = item.name;
          text.append(name);
          if (item.username) {
            const user = document.createElement("span");
            user.className = "user";
            user.textContent = item.username;
            text.append(user);
          }
          row.append(avatar, text);
          row.addEventListener("mousemove", () => active !== n && setActive(n));
          row.addEventListener("click", (e) => {
            if (trusted(e) && safeClick(e)) choose(n);
          });
          return row;
        }),
      );
      setActive(0);
    }
    place();
  }

  async function choose(n) {
    const item = entries[n];
    const el = field;
    if (!item || !el || busy) return;
    busy = true;
    try {
      const creds = await chrome.runtime
        .sendMessage({ type: "inline-fill", id: item.id })
        .catch(() => null);
      if (!creds || creds.error || creds.locked) {
        const why = !creds || appMissing(creds) ? tr("noAnswer") : creds.locked
          ? tr("inlineLockedRetry")
          : creds.error;
        note(tr("inlineFillFailed"), ` ${why}`, true);
        return;
      }
      if (typeof scoutVaultFillPage === "function") {
        scoutVaultFillPage(creds.username || "", creds.password || "", el);
      }
      hideAll();
    } finally {
      busy = false;
    }
  }

  // ---- Click safety --------------------------------------------------------

  const trusted = (e) => e.isTrusted === true;

  // The page can't hide our interface (opacity, overlay) to
  // steal a click, nor push the click right as the menu appears.
  function safeClick(e) {
    if (Date.now() - shownAt < CLICK_DELAY_MS) return false;
    if (getComputedStyle(host).opacity !== "1") return false;
    for (let n = document.documentElement; n; n = n.parentElement) {
      if (getComputedStyle(n).opacity !== "1") return false;
    }
    return document.elementFromPoint(e.clientX, e.clientY) === host;
  }

  // Clicking the menu doesn't take focus away from the field.
  root.addEventListener("mousedown", (e) => e.preventDefault());
  icon.addEventListener("click", (e) => {
    if (!trusted(e) || !safeClick(e)) return;
    if (menu.classList.contains("show")) closeMenu();
    else openMenu(false);
  });
  $(".foot").addEventListener("click", async (e) => {
    if (!trusted(e) || !safeClick(e)) return;
    const s = await chrome.storage.local.get({ inlineOff: [] }).catch(() => ({ inlineOff: [] }));
    if (!s.inlineOff.includes(location.hostname)) s.inlineOff.push(location.hostname);
    await chrome.storage.local.set({ inlineOff: s.inlineOff }).catch(() => {});
    hideAll();
  });

  // ---- Focus and keyboard --------------------------------------------------

  document.addEventListener(
    "focusin",
    (e) => {
      const el = e.composedPath()[0];
      if (isLoginField(el)) showIcon(el);
      else if (el !== host) hideAll();
    },
    true,
  );
  document.addEventListener(
    "focusout",
    () => {
      // Wait for focus to land: a click on our menu doesn't count (mousedown prevented).
      setTimeout(() => {
        if (field && document.activeElement !== field) hideAll();
      }, 0);
    },
    true,
  );

  addEventListener(
    "keydown",
    (e) => {
      if (!trusted(e)) return;
      const swallow = () => {
        e.preventDefault();
        e.stopImmediatePropagation();
      };
      // Alt+↓ works even before the icon appears (the app's answer still on its way).
      if (e.key === "ArrowDown" && e.altKey && !menu.classList.contains("show")) {
        if (e.target !== field && !isLoginField(e.target)) return;
        swallow();
        settings().then(({ on }) => {
          if (!on || document.activeElement !== e.target) return;
          field = e.target;
          attach();
          openMenu(false);
        });
        return;
      }
      if (!field || e.target !== field) return;
      if (!menu.classList.contains("show")) return;
      if (e.key === "Escape") {
        swallow();
        closeMenu();
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        swallow();
        setActive(active + (e.key === "ArrowDown" ? 1 : -1));
      } else if (e.key === "Enter" && active >= 0 && entries.length) {
        swallow();
        choose(active);
      } else if (e.key === "Tab") {
        closeMenu();
      }
    },
    true,
  );

  // Browser shortcut (chrome.commands → background): opens on the focused field
  // or on the first visible login field.
  chrome.runtime.onMessage.addListener((msg) => {
    if (!msg || msg.type !== "open-inline") return;
    let el = document.activeElement;
    if (!isLoginField(el)) {
      el = [...document.querySelectorAll("input")].find((i) => isLoginField(i) && visible(i));
    }
    if (!el) return;
    el.focus();
    settings().then(async ({ on }) => {
      // The shortcut works even with the icon turned off: it was asked for.
      field = el;
      attach();
      icon.classList.toggle("show", on);
      await openMenu(true);
    });
  });

  // The setting changed (popup): hide right away.
  chrome.storage.onChanged.addListener((changes) => {
    if (changes.inlineMenu || changes.inlineOff) {
      settings().then(({ on }) => !on && hideAll());
    }
  });

  // Page that swaps the form without reloading (SPA): the field is gone.
  new MutationObserver(() => field && !field.isConnected && hideAll()).observe(
    document.documentElement,
    { childList: true, subtree: true },
  );
})();
