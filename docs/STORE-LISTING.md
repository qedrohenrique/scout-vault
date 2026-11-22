# Browser store listings

What to fill in when publishing the extension. The packages come from each release
(`scout-vault-extension-<version>-chromium.zip` and `-firefox.zip`, built by
`tools/package_extension.py`).

## After publishing

The Chrome Web Store and Edge Add-ons give the extension an **id of their own** (the store
package has no `key`). Add each id to `STORE_EXTENSION_IDS` in
`crates/scout-vault-app/src/browser.rs` and release the app, or the bridge refuses the store
version. Firefox keeps `scout-vault@qedrohenrique.github.io` (`FIREFOX_EXTENSION_ID`).

## Listing

- **Name:** Scout Vault
- **Category:** Productivity (Chrome) / Privacy & Security (Firefox, Edge)
- **Privacy policy URL:** https://github.com/qedrohenrique/scout-vault/blob/main/docs/PRIVACY.md
- **Homepage / support:** https://github.com/qedrohenrique/scout-vault

**Short description (EN):** Fill logins, cards and addresses from the Scout Vault app, an
unofficial Bitwarden-compatible password manager for Windows.

**Short description (PT-BR):** Preencha logins, cartões e endereços com o app Scout Vault, um
gerenciador de senhas para Windows compatível com o Bitwarden (não oficial).

**Description (EN):**
Scout Vault's extension fills your logins, cards and identities in the browser from the Scout
Vault desktop app (required, Windows). It talks only to the app on your computer — never to the
internet — and a password reaches a page only after you click it, and only on the site saved
in the item. It can also offer to save a login you just used. Unofficial project, not affiliated
with Bitwarden Inc.

## Permission justifications (Chrome Web Store asks for each)

| Permission | Why |
|---|---|
| `nativeMessaging` | The only channel: asks the Scout Vault app on this computer for logins and sends the ones to save. |
| `activeTab`, `scripting` | Fill the form of the tab you're on, after you choose a login, card or identity in the popup. |
| `storage` | Remember the extension's own preferences, and keep a login you just submitted for up to 2 minutes until you choose whether to save it. |
| Host access `http://*/*`, `https://*/*` | The in-field menu and "save this login?" need to see login forms on the sites you visit. Nothing is read from pages you don't interact with, and nothing leaves the computer. |

**Single purpose:** fill and save logins, cards and identities from the user's own Scout Vault app.

**Data use disclosures:** collects no user data; no remote code; data isn't sold or transferred.
