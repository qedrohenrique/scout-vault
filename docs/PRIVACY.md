# Privacy policy — Scout Vault (app and browser extension)

Scout Vault is an unofficial, open-source password manager for Windows that works with a
Bitwarden-compatible server (Bitwarden's cloud or your own Vaultwarden). This page covers the
desktop app and the browser extension.

## What we collect

**Nothing.** The project runs no servers of its own, has no analytics, no telemetry and no crash
reporting. Nothing you store in Scout Vault is sent to the project or its developers.

## Where your data goes

- **To the server you chose** (Bitwarden's cloud or your Vaultwarden), exactly as the official
  Bitwarden apps do: your vault travels **encrypted** on your computer before it leaves, with
  keys derived from your master password, which never leaves your computer.
- **Website icons** (optional, on by default): the app asks the same server's icon service for
  the icons of the sites in your vault, by site name. Turn it off in Settings.
- **Update checks** (optional, on by default): the app asks GitHub (github.com) whether there is
  a newer Scout Vault release. No vault data goes with it. Turn it off in Settings → About.
- **Have I Been Pwned** (only when you ask, in "Password health"): the first 5 characters of a
  password's SHA-1 hash are sent (k-anonymity); the password itself never is.

## The browser extension

- Talks **only to the Scout Vault app on the same computer** (Chrome/Edge/Firefox native
  messaging), never to the internet.
- Reads the page's address and its form fields to offer and fill your logins, cards and
  identities. A password, card or identity reaches a page **only after you click it**, and
  logins only on the site saved in the item.
- When you submit a login form, it may offer to save it; the login stays in the browser's memory
  for at most 2 minutes, until you answer.
- Stores in the browser only its own preferences (the in-field menu on/off, sites where you hid
  it).

## On your computer

The app keeps its settings in `%APPDATA%\Scout Vault` and, if you choose "remember session",
an encrypted copy of your vault (the same ciphertext the server keeps). Details, and what is
protected against what, in the [threat model](THREAT-MODEL.md).

## Contact

Questions or concerns: open an issue at
https://github.com/qedrohenrique/scout-vault/issues — or, for anything security related,
report it privately as explained in [SECURITY.md](../SECURITY.md).
