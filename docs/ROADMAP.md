# Roadmap — Scout Vault

Native desktop client (Rust + GPUI) for Bitwarden-compatible servers (official Bitwarden and
Vaultwarden). Goal: a **light, fast and pleasant** app that replaces the Electron desktop app for
daily use — and goes beyond it where a native desktop app shines (keyboard, Windows, local
privacy).

**Principles**
- Nothing decrypted goes to disk (explicit, optional exception: the user key in Credential
  Manager for "open without password for N days"; with Windows Hello it is encrypted by Hello).
  Secrets live in memory with `zeroize`.
- `scout-vault-core` knows nothing about the UI: protocol and cryptography are testable on their own.
- Development against a **local Vaultwarden** (`dev/`); a real account only for manual validation.
- Implement from the specifications and from MIT projects (e.g. [`rbw`](https://github.com/doy/rbw));
  **do not copy code from the official SDK** (restrictive license).
- Interoperate without losing data: whatever Scout Vault doesn't understand goes back to the
  server untouched.

---

## Where we are (0.2.x)

| Area | Done |
|---|---|
| **Account** | Login (PBKDF2/Argon2id), 2FA (authenticator, email, YubiKey, FIDO2/Windows Hello, Duo), new device, "remember device", SSO with master password, v2 accounts (COSE), captcha/rate limits explained |
| **Session** | Remembered session and encrypted cache (opens offline), open without password for N days, full login every N days, Windows Hello unlock, background sync, configurable idle lock, lock together with Windows (Win+L and sleep), password change detected |
| **Vault** | All item types, search, filters, folders (CRUD), organizations and collections (create/choose/change), quick access (pinned items), live TOTP, trash, attachments (upload/open/save, drag and drop), password history preserved |
| **Writing** | Create/edit/trash/restore/delete, preserving passkeys, URI rules and unknown fields |
| **Windows** | Fill in apps (`Ctrl+Alt+A`), tray (close ≠ quit), single instance, clipboard kept out of Win+V, executable icon/resources |
| **Browser** | MV3 extension (Chrome/Edge/Brave) with native messaging and a pipe restricted to the user |
| **Interface** | Light/dark theme, accent color, animations (with "Still"), toasts, password generator, shortcuts |
| **Quality** | Headless UI tests (including layout), public crypto test vectors, live tests against Vaultwarden |

Manual validation with a real account: test plan in [REAL-ACCOUNT-TESTING.md](REAL-ACCOUNT-TESTING.md).

---

## Market comparison

Overview (Nov 2026) based on each product's public documentation, looking only at the **desktop
app + extension** on Windows. Features vary by plan; ✅ has it, 🟡 partial, ❌ doesn't have it.

| Feature | Scout Vault | Bitwarden (official) | 1Password | KeePassXC | Proton Pass |
|---|:-:|:-:|:-:|:-:|:-:|
| Light native app (no Electron) | ✅ | ❌ | ❌ | ✅ | ❌ |
| Cloud sync / multi-device | ✅ (Bitwarden server) | ✅ | ✅ | ❌ (file) | ✅ |
| Browser autofill | 🟡 popup | ✅ inline + automatic | ✅ | ✅ | ✅ |
| Save/update login from the browser | ✅ | ✅ | ✅ | ✅ | ✅ |
| Fill in desktop apps (auto-type) | ✅ | ❌ | 🟡 | ✅ | ❌ |
| Passkeys (create and use) | ❌ (preserved) | ✅ | ✅ | ✅ | ✅ |
| Global quick search (system shortcut) | ✅ | ❌ | ✅ Quick Access | 🟡 | ❌ |
| Unlock with Windows Hello | ✅ | ✅ | ✅ | ✅ | ✅ |
| Lock together with Windows | ✅ | ✅ | ✅ | ✅ | ✅ |
| Password re-prompt on sensitive items | ✅ | ✅ | 🟡 | ❌ | ❌ |
| Password health (weak, reused, leaked) | ✅ | ✅ reports | ✅ Watchtower | ✅ | ✅ |
| Passphrase and username generator | 🟡 password only | ✅ | ✅ | ✅ | ✅ |
| Email aliases (hide my email) | ❌ | ✅ via services | ✅ | ❌ | ✅ native |
| Secure sharing (Send / link) | ❌ | ✅ Send | ✅ | ❌ | ✅ |
| Move item to organization / share | ❌ | ✅ | ✅ | ❌ | ✅ |
| Item version history | 🟡 password only | 🟡 password only | ✅ | ✅ | ❌ |
| Several accounts at once | ❌ | ✅ | ✅ | ✅ (several files) | ✅ |
| SSH agent | ❌ (stores keys) | ✅ | ✅ | ✅ | ❌ |
| Import / export | ❌ | ✅ | ✅ | ✅ | ✅ |
| Emergency access | ❌ | ✅ | 🟡 | ❌ | ❌ |
| Website icons in the list | ❌ | ✅ | ✅ | ✅ | ✅ |
| Firefox | ❌ | ✅ | ✅ | ✅ | ✅ |
| Installer / automatic updates | ❌ (zip) | ✅ | ✅ | ✅ | ✅ |
| Linux / macOS | ❌ | ✅ | ✅ | ✅ | ✅ |
| External security audit | ❌ | ✅ | ✅ | ✅ | ✅ |

**Where we already stand out:** lightness and speed, real auto-type, tray/single instance,
clipboard kept out of history, animated and keyboard-accessible interface, Vaultwarden as a
first-class citizen.

**Gaps that matter most day to day:** inline browser autofill, passkeys, password health,
installer (Windows Hello, locking with Windows, re-prompt and saving logins are already in).

---

## Next phases (priority from top to bottom)

### Phase A — Everyday security
- ~~Unlock with Windows Hello~~ — done: the user key is encrypted by a Hello signature (key in the TPM), with the master password as a fallback.
- ~~Lock when Windows locks/sleeps~~ — done (option "Lock together with Windows", on by default).
- ~~Master password re-prompt~~ — done: marked items ask for the password to reveal, copy, edit and type; the browser doesn't receive these items.
- ~~Lock on minimize and clipboard timeout~~ — done: "Lock on minimize" (also when closing to the tray) and "Clear the clipboard after" 10 s to 2 min, or never.
- ~~Background sync~~ — done: every minute the app queries `accounts/revision-date` (cheap) and syncs only if the vault changed. Next step: real-time notifications (`/notifications/hub`, SignalR over WebSocket).

### Phase B — First-class browser
- ~~Save and update logins~~ — done: a "Save/Update in Scout Vault?" bar after the form is submitted; the app decides between new and update.
- ~~Inline menu~~ — done: icon on the edge of the login field with the number of logins; click,
  `Alt+↓` or `Alt+Shift+L` open the list; "Don't show on this site" and a switch in the popup.
  Missing: forms inside iframes (today only the main page).
- Filling **cards and identities**; copy the TOTP automatically after filling.
- **Firefox** (its own manifest, `allowed_extensions`).
- Publish to the stores (Chrome Web Store, Edge Add-ons) so developer mode isn't needed.

### Phase C — Desktop productivity
- ~~Global quick search~~ — done: `Ctrl+Shift+Space` opens a floating window to find, copy and fill without opening the app.
- **Website icons** in the list (the server's icon service, with a cache and an option to turn it off).
- Auto-type: **per-item sequence** (`{USERNAME}{TAB}{PASSWORD}{ENTER}`, `{TOTP}`, delays) and admin windows.
- Configurable shortcuts; command palette (`Ctrl+K`).
- Custom title bar that follows the theme.

### Phase D — Health and generation
- ~~Health report~~ — done: "Password health" screen (`Ctrl+Shift+H`) with a score, weak,
  reused, old (no change in over a year) and **leaked** passwords (Have I Been Pwned with
  k-anonymity: only 5 characters of the SHA-1 leave the machine, and only when the person asks).
  Missing: sites that support 2FA where the item has none (2fa.directory list).
- **Passphrases** (EFF list, CC BY 3.0) and **username generator**.
- **Email aliases** via SimpleLogin, addy.io, Fastmail and DuckDuckGo (as Bitwarden does).

### Phase E — Passkeys
- Store and use **passkeys** through the extension (WebAuthn with vault credentials, `fido2Credentials` format).
- Later: Windows passkey provider (Windows 11 plugin API), if stable.

### Phase F — Sharing and collaboration
- **Move item to organization** (re-encrypt with the org key, attachments included).
- **Bitwarden Send** (text and file, with expiry, password and access limit).
- **Several accounts** open with quick switching between them.
- Emergency access (accept/grant) — depends on server support.
- New server item types (bank account, driver's license, passport) once stable.

### Phase G — Data
- **Import**: Bitwarden JSON/CSV, 1Password `.1pux`, KeePass `.kdbx`/CSV, Chrome/Edge CSV, Proton Pass.
- **Export**: password-encrypted JSON (Bitwarden format) and CSV (with a warning).
- Item version history (local, with restore).

### Phase H — Distribution and trust
- **Installer** (MSI/winget), **automatic updates** with signed updates.
- **Code signing** of the executable (removes the SmartScreen warning).
- Builds for **Linux** and **macOS** (GPUI already runs there).
- **External security review** and vulnerability disclosure policy.
- ~~English interface (i18n)~~ — done: pt-BR and English in the app and the extension, with
  catalogs that accept new languages.
- Accessibility review (screen reader, contrast).
- SSH agent (the vault already stores the keys).

---

## Risks and open decisions
- **API not formally documented** — the server changes requirements (e.g. minimum client
  version, `encryptedFor`); follow `rbw`, the Vaultwarden changelog and the official clients.
- **Uploading attachments to the official cloud (Azure)** not yet validated with a real account.
- **Security review** — before recommending it for wide use.
- **Branding** — don't use the "Bitwarden" name/logo; "Bitwarden compatible", unofficial project.
- **"Open without password for N days"** trades convenience for local risk; it is off by default
  and discarded when the password changes.
