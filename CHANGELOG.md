# Changelog

Every change that matters to Scout Vault users. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Downloads are on the
[GitHub Releases](https://github.com/qedrohenrique/scout-vault/releases) page.

## [Unreleased]

## [0.6.0] - 2026-11-17

### Added
- **Shortcuts in settings:** the global shortcuts (fill in other apps, quick search) can be
  changed — click, press the new combination, done. Esc cancels; combinations without
  Ctrl/Alt/Win or already used by the other shortcut are refused; "Back to default" undoes.
- **Profile screen** (click your email at the bottom of the sidebar): name, email and badges
  (Premium, email verified, 2FA), server, sign-in method, account creation date, this device,
  organizations, the vault in numbers and what this computer remembers. From there: open the
  web vault (to change password, email or 2FA) or sign out.

## [0.5.0] - 2026-11-16

### Added
- **Auto-type checks the field before typing the password:** Windows UI Automation must say the
  focused field is a password field, or Scout Vault stops (the cursor could be in a chat box).
  Apps that don't tell their fields get a confirmation in the HUD before anything is typed.

### Changed
- The HUD fits its content (a short list makes a short window) and, like the tray menu, is
  translucent over a blurred backdrop. **Settings → Appearance → Floating windows opacity** is a
  slider from 1% to 100% (default 92%).
- The window's title bar (minimize, maximize, close) is drawn by the app in the screen's own
  colors — the sidebar's column continues up to the top — so window and app read as one piece.
  The logo sits in it, over the sidebar, and the vault's actions (sync, password health,
  generator, settings, lock) moved from the sidebar's footer to its right side.
- In the item icon picker, the colors come first, above the icons and brands.
- Settings: a **Back** button fixed on the left replaces the close button at the top, so you
  don't have to scroll back up to leave.
- Signed out, `Ctrl+Alt+A` and quick search show a compact login (2FA, new-device code, SSO)
  inside the HUD instead of the main window.
- Auto-type messages (stopped, couldn't type) show up in the HUD, not as a toast in the
  main window.

### Fixed
- The HUD and the tray menu no longer show a dark, blurred margin around the card: the window is
  now exactly the card's size, with corners rounded by Windows.

## [0.4.1] - 2026-11-16

### Changed
- **Auto-type HUD:** when `Ctrl+Alt+A` can't pick the login from the window title, a floating
  HUD opens apart from the main window, with only the search, the logins and a close button.
- With the vault locked, `Ctrl+Alt+A` and quick search no longer bring up the main window:
  within the "ask for the password on open" period they open without asking; otherwise the HUD
  itself asks for the vault password.
- The item icon picker closes when you click outside it.

## [0.4.0] - 2026-11-16

### Added
- **Global quick search** (`Ctrl+Shift+Space`): from any app, a small window to find an item
  and fill it (`Enter`), copy the password, username or code (`Ctrl+P`/`Ctrl+U`/`Ctrl+T`) or
  open it in Scout Vault (`Ctrl+O`). Items that ask for the vault password still ask.
- **Menu in login fields** (extension): the Scout Vault icon shows up in the username or
  password field; a click, `Alt+↓` or `Alt+Shift+L` show the site's logins to fill right there.
  It can be hidden per site or turned off in the popup.
- **English interface**: the app and the extension now speak Portuguese (pt-BR) and English. The
  app follows the Windows language and can be switched in **Settings → Language**, instantly.
- **Password health** (`Ctrl+Shift+H`): a score plus weak, reused and old passwords, and an
  on-demand check against Have I Been Pwned. Only the first 5 characters of each password's SHA-1
  hash leave the computer (k-anonymity).
- **Item icons:** click the icon in an item's detail to pick one of 50+ icons and a color. Your
  choice stays on this computer; the item doesn't change on the server.
- **Brand icons:** 46 well-known services (Google, Claude, Riot Games, Valorant, Discord,
  Instagram, Steam, Nubank…) from Simple Icons (CC0). Logins whose address belongs to one of them
  show the brand on their own; a manual choice still wins.
- **Clear the clipboard after** 10 s, 30 s, 1 min, 2 min or never (Settings).
- **Lock on minimize** (Settings), also when the window closes to the tray.

### Changed
- Texts reviewed to be shorter and more direct; "master password" became **vault password**.
- GitHub Actions updated (checkout, setup-python, upload/download-artifact, CodeQL).
- Repository documentation, scripts and code comments translated to English.
- New **monochrome logo**: graphite tile with a white glyph, in the app, the executable, the tray,
  the extension and the README.
- Item names, labels and section titles use a slightly heavier font weight.
- The auto-type picker (`Ctrl+Alt+A`) and quick search show each item's custom icon and color.

### Fixed
- Tray menu: no more darker frame around the card, and the header's color no longer leaks past
  the rounded corners.
- The entry screen's animated background no longer freezes when the window loses focus (it
  still stops while minimized).

## [0.3.0] - 2026-11-15

### Added
- **Save logins from the browser:** after signing in to a site, the extension asks
  "Save this login in Scout Vault?" or "Update the password for …?". Nothing is saved without
  the click; the old password goes to the history. With the vault locked, the app asks for the
  password and saves right after. The extension now runs on all sites (the browser warns when
  updating).
- **Unlock with Windows Hello** (face, fingerprint or PIN). The vault key is encrypted by a
  Windows Hello signature; changing the master password or signing out removes the setup.
- **Background sync:** every minute the app asks the server whether the vault changed and, if it
  changed on another device, updates on its own.
- **Ask for the master password** on marked items (as in Bitwarden): show, copy, edit and type
  wait for the confirmation; the extension doesn't fill these items.
- **Lock together with Windows** (Win+L, sleep and hibernation).
- **Tray menu** with quick access to pinned items (copy username, password or code),
  "Generate and copy password" and "Sync now".
- **Items in organizations:** choose the organization and the collections in the form;
  collections in the sidebar.
- **Configurable session:** open without a password for N days, ask for a full login every N
  days and idle lock time.
- **Drag and drop:** items into folders and folders into other folders (subfolders).
- `CHANGELOG.md` (this file) and automatic releases: pushing a `vX.Y.Z` tag tests, builds,
  packages and publishes the release with that version's notes.

### Changed
- The tray's right-click menu is drawn by the app itself, with the theme and the accent color.
- Changing the theme, accent color, text size or corners transforms the interface in place: each
  color goes from the old one to the new one, and between light and dark a wave starts from the
  clicked button ("Motion: Still" switches directly).
- Clicking an item in the list shows the item even with a form open.
- Roadmap redone with a comparison against Bitwarden, 1Password, KeePassXC and Proton Pass.
- CI on GitHub Actions: formatting, clippy and tests on Windows, core tests against a
  throwaway Vaultwarden, cargo-deny, CodeQL and Dependabot.

## [0.2.0] - 2026-11-14

### Added
- **Folders:** create, rename and delete from the sidebar.
- **Quick access:** items pinned to the top of the sidebar.
- **Easier attachments:** click or drag files in the form and in the item detail; a clear
  message when the account can't upload attachments (Premium).
- **Windows tray:** closing the window keeps the app running; opening it again just shows the
  window.
- **Toasts** for notices and errors.
- **Animations** on screens, sidebar, selection, sync and loading
  (Settings → Motion → Still turns them off).

### Changed
- More compact form: short fields side by side and examples inside the fields.

## [0.1.0] - 2026-11-14

First test version.

### Added
- Login with email and master password, 2FA (authenticator, email, YubiKey, security
  key/Windows Hello, Duo) and company SSO; bitwarden.com, bitwarden.eu or Vaultwarden.
- Remembered session: opens asking only for the master password, even without internet.
- Search, filters, live authenticator code; copies disappear after 30 s and stay out of Win+V.
- Create and edit logins, notes, cards and identities; trash; password history; data from
  other apps (passkeys, URI rules) preserved.
- Attachments: attach, open, save and remove.
- Fill in Windows apps with `Ctrl+Alt+A`.
- Extension for Chrome, Edge and Brave that fills logins.
- Password generator, light/dark themes and accent color.

[Unreleased]: https://github.com/qedrohenrique/scout-vault/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/qedrohenrique/scout-vault/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/qedrohenrique/scout-vault/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/qedrohenrique/scout-vault/releases/tag/v0.1.0
