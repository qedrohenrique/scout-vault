# Scout Vault — threat model

What Scout Vault protects, from whom, how, and what it **doesn't** protect against. Written for an
external security review (a 1.0 blocker, see the [roadmap](ROADMAP.md#road-to-10)) and for anyone
deciding whether to trust the app. Keep it in step with the code: each claim names the module that
implements it.

## What is protected

| Asset | Where it lives | Notes |
|---|---|---|
| Master password | Typed, kept only while deriving keys | Never stored, never sent: the server gets a PBKDF2 hash of the master key (`core::crypto`). |
| Master key / user key / organization keys | Memory while unlocked (`Zeroizing`) | Dropped on lock; the user key can also be stored by the options below. |
| Decrypted vault (items, attachments) | Memory while unlocked | Wiped when the vault screen goes away (lock, sign out). |
| Encrypted vault (sync response) | `%APPDATA%\Scout Vault\sessao.json` (remembered session) | Same ciphertext the server keeps; opens only with the master password. |
| Refresh token, 2FA "remember device" token | Windows Credential Manager | `saved_session`, `secrets`. |
| User key for "open without password for N days" | Windows Credential Manager, with an expiry | `quick_unlock`; off by default. |
| User key for Windows Hello | Credential Manager, **encrypted** with a key derived from a TPM-backed Hello signature | `hello`. |
| Item metadata on disk | `settings.json`: pinned item ids, per-item icon/color, sidebar choices | Ids and appearance only — no names, URLs or secrets. |
| Profile photo | `%APPDATA%\Scout Vault\photos\<hash of email>` | Chosen by the person; not secret. |

## Who we defend against

1. **A compromised or malicious server** (or someone in the middle of TLS). It must not learn the
   master password or decrypt the vault, and must not weaken the key derivation: the app refuses
   KDF parameters below Bitwarden's minimums (`core::kdf`). It **can** deny service, withhold
   updates to items, or serve stale data.
2. **Another Windows user** on the same computer. The local pipes (browser bridge, SSH agent) have
   an ACL for the current user only and refuse remote clients (`browser::server`, `ssh_agent`).
   Credential Manager entries belong to the Windows user.
3. **Other programs running as the same user, without admin rights** (malware the person ran, a
   malicious browser extension). **Only partially, and mostly not:** such a program can read the
   app's memory while it is unlocked, and it can talk to the local pipes like a legitimate
   client. The protections below raise the cost or make it visible; they are not a boundary.
   See *Residual risks*. What we do:
   - **Browser pipe.** The browser only starts the bridge for the registered extension id, and
     the *extension* only fills a tab whose site matches the item's address, after the person
     clicks. But the pipe can't tell the extension from another program of the same user: such
     a program can ask for the logins of any site it names, and for cards (with the security
     code) and identities, without a click. The app limits requests that hand out a secret to
     12 a minute (lists to 120) and warns when the limit trips, and a request through the pipe
     never postpones the idle lock (`browser::Limits`, `app::handle_browser`).
   - The SSH agent asks before every signature, naming the program by its **full path** and its
     parent ("Remember" is per path, for 10 minutes); keys whose item asks for the master
     password need it for each signature; it refuses to add, remove or export keys
     (`ssh_agent`, `core::ssh_agent`). A program of the same user can still click "Allow"
     itself (synthetic input); Windows Hello confirmation is the stronger option.
   - **Auto-type** always asks in the HUD before typing, even with a single match, and names
     the target **program** next to the window title, with a warning in web browsers (where
     the title is the page's `<title>`, which any site chooses). Before every part it checks
     the focused window is still the target and, before the password, that the focused field
     **is a password field** (UI Automation) (`autotype`, `quick_search`).
   - Copied secrets are marked to stay out of clipboard history and the cloud clipboard, and the
     clipboard clears itself after a configurable time (`clipboard`), unless the app is closed
     first.
4. **Someone with the unlocked computer for a moment.** Idle lock, lock with Windows (Win+L,
   sleep), lock on minimize, and re-prompt (master password) for marked items before revealing,
   copying, editing, typing or exporting. Export always asks for the master password.
5. **Someone with the powered-off disk** (stolen laptop without disk encryption). What's on disk is
   ciphertext, except the items listed in *Residual risks* (quick unlock, attachment temp files).
6. **A tampered update.** The updater only downloads from the project's own GitHub release URLs,
   checks the published SHA-256 and runs the installer only if its **Ed25519 signature** matches
   the public key built into the app (`core::update::RELEASE_PUBLIC_KEY`). The signature covers
   the file's **name, version and SHA-256** (`core::update::release_message`), so an old signed
   installer can't be offered as a newer version (no downgrade by editing a release), and from
   1.0 on pre-releases are only offered to people already on one. The private key is a secret
   of the `release` environment, used only by a separate workflow job that compiles and
   installs nothing (`tools/sign_release.sh`); the build job never sees it. Replacing or
   renaming files on the release page isn't enough to push an update. Apps up to 0.13 check an
   older signature over the bytes only, so they stay open to such a downgrade until they update.

## Not in scope

- Malware running as **administrator / SYSTEM**, or a kernel/driver compromise: it can read any
  process's memory, including the unlocked vault. No desktop password manager defends this.
- Keyloggers capturing the master password as it is typed.
- A compromised Windows account (the attacker *is* the user): Credential Manager entries open for
  them, and so does the quick-unlock key.
- Physical attacks on RAM (cold boot), side channels.
- Bugs in the Bitwarden server, Vaultwarden, Windows, the browser or GPUI themselves.

## Residual risks (known, accepted for now)

- **Quick unlock** ("open without password for N days") stores the user key in Credential
  Manager: anyone logged in as the user — or malware running as them — opens the vault without
  the password until it expires. Off by default; discarded when the master password changes.
- **Opening an attachment in another program** writes it decrypted to
  `%TEMP%\ScoutVault-anexos\<random>\` (with Mark-of-the-Web); it is deleted when the vault locks
  or on the next start after a crash, but the other program may keep its own copies (recent
  files, thumbnails). Attachments whose name is a program, script, shortcut or macro document
  are never opened (only "Save as"), and invisible direction characters are removed from names
  (`attachments`): the name is chosen by whoever attached it.
- **Exports** in plain JSON/CSV are unencrypted by design (with a warning); the password-protected
  JSON is the default.
- **Same-user malware** can read the process memory while unlocked, inject keystrokes (including
  answering the SSH agent's or the auto-type HUD's prompt), or talk to the pipes like a
  legitimate client: through the browser pipe it gets logins of any site it names, cards and
  identities, at the rate limit above. The installed app and the unpacked extension live in
  folders the user can write to, so such malware can also replace them. No desktop password
  manager defends against this; we only make it slower and louder.
- **Moved fields.** The Bitwarden format doesn't bind an encrypted value to its field: a
  malicious server can swap values inside an item (for example, show the password as the
  username) and, for items without their own key (items Scout Vault creates don't have one
  yet), between items. It can't read them. Website icons are only asked for addresses with an
  explicit `http(s)://`, so a moved secret isn't sent to the icon service.
- **Key derivation has minimums but no maximum**: a malicious server (or an imported file) can
  ask for very expensive parameters and make unlocking hang.
- **Windows Hello unlock** uses a key in the TPM when the computer has one, but that isn't
  checked; the Hello credential belongs to the Windows user.
- **Not every decrypted value is wiped** on lock: secrets (`Secret`) are zeroed, but names,
  usernames, addresses and other plain fields may stay in freed memory until reused.
- **Credential Manager entries** (refresh token, quick unlock, Hello) use the default
  persistence (they can roam with a roaming profile) and aren't removed by the uninstaller; an
  expired quick-unlock key stays stored until the next unlock attempt.
- **Browser extension**: a page can tell that Scout Vault is installed (the save bar), and
  learns whether a password it submits is already saved (the bar doesn't show); a login saved
  for `https://` is offered on `http://` pages of the same site; WebAuthn's `rpId` from the
  server isn't checked against the vault's address.
- The executable and installer aren't **Authenticode-signed** yet (SmartScreen warning, and a
  first install downloaded by hand has nothing but the `.sha256`/`.sig` to check). Whoever holds
  the release signing key (the GitHub secret and the owner's backup) can sign updates.
- **Website icons** are fetched from the server's icon service by host name: the server learns
  which sites are in the vault (as with Bitwarden's apps). The option can be turned off.
- **Update checks** tell GitHub the app is running (an ordinary request, no vault data). Can be
  turned off.

## Where to look first (for a reviewer)

1. `crates/scout-vault-core/src/crypto.rs`, `enc_string.rs`, `cose.rs` — key derivation, AES-CBC +
   HMAC (constant-time MAC check before decrypting), RSA-OAEP, item keys, v2 (COSE) accounts.
2. `crates/scout-vault-core/src/edit.rs` — encrypting edited items without dropping unknown data.
3. `crates/scout-vault-app/src/quick_unlock.rs`, `hello.rs`, `saved_session.rs`, `secrets.rs` —
   what reaches Credential Manager and when it is discarded.
4. `crates/scout-vault-app/src/browser.rs` and `extension/` — native messaging, the pipe's ACL,
   which data reaches a tab and when.
5. `crates/scout-vault-app/src/ssh_agent.rs`, `core/src/ssh_agent.rs` — the agent's pipe, client
   identification and approval.
6. `crates/scout-vault-app/src/autotype.rs` — the checks before typing.
7. `crates/scout-vault-app/src/updater.rs`, `core/src/update.rs`, `installer/scout-vault.iss` —
   what the updater trusts.
8. `core/src/import.rs`, `core/src/export.rs` — parsing untrusted files; the password-protected
   format.
