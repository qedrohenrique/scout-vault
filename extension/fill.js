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
