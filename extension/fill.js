// Preenche usuário e senha na página. Injetada pelo popup com
// chrome.scripting.executeScript({ func: cofreFillPage, args: [...] }):
// precisa ser autocontida (o navegador serializa só o corpo da função).
function cofreFillPage(username, password) {
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
    // O setter nativo dispara o que frameworks (React, Vue) observam.
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
    setter.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  };

  const inputs = [...document.querySelectorAll("input")].filter(visible);
  const password_field = inputs.find((i) => i.type === "password") || null;
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
  } else {
    // Login em duas etapas: só o usuário nesta página.
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
