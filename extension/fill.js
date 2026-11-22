// Fills username and password on the page. Injected by the popup with
// chrome.scripting.executeScript({ func: scoutVaultFillPage, args: [...] }):
// it must be self-contained (the browser serializes only the function body).
// The inline menu (inline.js) passes `anchor`, the field the person clicked: then
// that field's form applies, not the first one on the page.
function scoutVaultFillPage(username, password, anchor) {
  const visible = (el) => {
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return (
      r.width > 0 &&
      r.height > 0 &&
      s.visibility !== "hidden" &&
      s.display !== "none" &&
      !el.disabled &&
      !el.readOnly
    );
  };
  const describe = (i) =>
    `${i.name || ""} ${i.id || ""} ${i.getAttribute("autocomplete") || ""} ${i.placeholder || ""}`;
  const isUserLike = (i) =>
    ["text", "email", "tel", ""].includes(i.type) &&
    !/search|captcha|otp|one-time|\bcode\b|cupom|coupon/i.test(describe(i));
  const auto = (i) => (i.getAttribute("autocomplete") || "").toLowerCase();
  const set = (el, value) => {
    el.focus();
    // The native setter triggers what frameworks (React, Vue) observe.
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
    setter.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  };

  const root = (anchor && anchor.form) || document;
  let inputs = [...root.querySelectorAll("input")].filter(visible);
  if (!inputs.some((i) => i.type === "password") && root !== document && anchor.type !== "password") {
    // Form with only the username (two-step login) and the password outside it: the whole page.
    const all = [...document.querySelectorAll("input")].filter(visible);
    if (all.some((i) => i.type === "password")) inputs = all;
  }
  const password_field =
    (anchor && anchor.type === "password" && visible(anchor) ? anchor : null) ||
    inputs.find((i) => i.type === "password") ||
    null;
  let user_field = null;
  if (password_field) {
    const scope = password_field.form
      ? [...password_field.form.querySelectorAll("input")].filter(visible)
      : inputs;
    const before = scope.slice(0, scope.indexOf(password_field)).filter(isUserLike);
    user_field =
      scope.find((i) => /username|email/.test(auto(i)) && isUserLike(i)) ||
      before[before.length - 1] ||
      null;
  } else if (anchor && isUserLike(anchor) && visible(anchor)) {
    // Two-step login, starting from the chosen field.
    user_field = anchor;
  } else {
    // Two-step login: only the username on this page.
    user_field =
      inputs.find((i) => /username|email/.test(auto(i))) ||
      inputs.find((i) => isUserLike(i) && /user|email|login|mail|cpf/i.test(describe(i))) ||
      null;
  }

  const filled = { username: false, password: false };
  if (user_field && username) {
    set(user_field, username);
    filled.username = true;
  }
  if (password_field && password) {
    set(password_field, password);
    filled.password = true;
  }
  return filled;
}

// Fills a card or an identity (`kind`: "card" | "identity") on the page, by the fields'
// `autocomplete` and, failing that, their names/labels in Portuguese and English.
// Self-contained, like scoutVaultFillPage. Returns how many fields were filled.
function scoutVaultFillWallet(kind, data) {
  const usable = (el) => {
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none" && !el.disabled && !el.readOnly;
  };
  const labelOf = (el) => {
    const byFor = el.id ? document.querySelector(`label[for="${CSS.escape(el.id)}"]`) : null;
    const wrap = el.closest("label");
    return `${(byFor && byFor.textContent) || ""} ${(wrap && wrap.textContent) || ""}`;
  };
  const describe = (el) =>
    `${el.name || ""} ${el.id || ""} ${el.placeholder || ""} ${el.getAttribute("aria-label") || ""} ${labelOf(el)}`
      .toLowerCase()
      .normalize("NFD")
      .replace(/[\u0300-\u036f]/g, "");
  const auto = (el) => (el.getAttribute("autocomplete") || "").toLowerCase();
  const fields = [...document.querySelectorAll("input, select")].filter(
    (el) => usable(el) && !["hidden", "password", "checkbox", "radio", "submit", "button"].includes(el.type),
  );
  const used = new Set();
  const find = (tokens, pattern) =>
    fields.find((el) => !used.has(el) && tokens.some((t) => auto(el).split(/\s+/).includes(t))) ||
    (pattern ? fields.find((el) => !used.has(el) && !auto(el).startsWith("cc-") && pattern.test(describe(el))) : null) ||
    null;
  const write = (el, value) => {
    if (!el || value == null || value === "") return false;
    used.add(el);
    el.focus();
    if (el.tagName === "SELECT") {
      const want = String(value).toLowerCase();
      const option = [...el.options].find((o) => {
        const v = o.value.toLowerCase();
        const t = o.textContent.trim().toLowerCase();
        return v === want || t === want || Number(v) === Number(want) || (want.length === 4 && (v === want.slice(2) || t === want.slice(2)));
      });
      if (!option) return false;
      el.value = option.value;
    } else {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
      setter.call(el, String(value));
      el.dispatchEvent(new Event("input", { bubbles: true }));
    }
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  };
  let filled = 0;
  const put = (tokens, pattern, value) => {
    if (write(find(tokens, pattern), value)) filled += 1;
  };

  if (kind === "card") {
    const month = String(data.expMonth || "").padStart(2, "0");
    const year = String(data.expYear || "");
    put(["cc-number"], /(card.?num|numero.*cart|cartao|ccnum|card number|n.? do cartao)/, (data.number || "").replace(/\s+/g, ""));
    put(["cc-name"], /(titular|nome.*cart|name.*card|card.?holder|nome impresso)/, data.cardholderName);
    const combined = find(["cc-exp"], /(validade|expir|exp.?date|mm ?\/ ?(aa|yy))/);
    if (combined && combined.tagName !== "SELECT") {
      if (write(combined, `${month}/${year.slice(-2)}`)) filled += 1;
    } else {
      put(["cc-exp-month"], /(mes|month|exp.*mm)/, month);
      put(["cc-exp-year"], /(ano|year|exp.*(aa|yy))/, year);
    }
    put(["cc-csc"], /(cvv|cvc|csc|codigo de seguranca|security.?code|cod.? seg)/, data.code);
  } else {
    const full = [data.firstName, data.middleName, data.lastName].filter(Boolean).join(" ");
    put(["given-name"], /(primeiro nome|first.?name|nome(?!.*(sobre|ultimo|complet|usuario|empresa)))/, data.firstName);
    put(["additional-name"], /(nome do meio|middle.?name)/, data.middleName);
    put(["family-name"], /(sobrenome|ultimo nome|last.?name|surname)/, data.lastName);
    put(["name"], /(nome completo|full.?name)/, full);
    put(["email"], /(e-?mail)/, data.email);
    put(["tel"], /(telefone|celular|phone|mobile|whatsapp)/, data.phone);
    put(["organization"], /(empresa|company|organization)/, data.company);
    put(["address-line1", "street-address"], /(endereco|logradouro|rua|address.?(1|line)|street)/, data.address1);
    put(["address-line2"], /(complemento|address.?2|apartamento|apt)/, data.address2);
    put(["address-line3"], /(address.?3|bairro)/, data.address3);
    put(["address-level2"], /(cidade|municipio|city|town)/, data.city);
    put(["address-level1"], /(estado|uf\b|state|province|region)/, data.state);
    put(["postal-code"], /(cep|postal|zip)/, data.postalCode);
    put(["country", "country-name"], /(pais|country)/, data.country);
    put(["username"], /(usuario|username|user.?name|login)/, data.username);
  }
  return filled;
}
