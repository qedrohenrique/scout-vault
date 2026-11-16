# Test plan with a real account (bitwarden.com / bitwarden.eu)

Scout Vault creates, edits and deletes items on the server only when you ask it to. Even so,
start with a **test account**. Use your real account only after the test account passes.

## Before you start
1. Create a free account at https://vault.bitwarden.com with an email address you can read.
2. In the web vault, add a few items: a login with username, password and website; a card; a
   note; a folder; and an item marked as favorite.
3. Run the app with `cargo run`. On the login screen, choose **bitwarden.com** (or
   **bitwarden.eu**, if the account is European).

## Cases to test
Go through the cases in order. Write down the exact text that shows up below the form.

| # | Scenario | What to do | Expected |
|---|---|---|---|
| 1 | Wrong password | Sign in with the wrong password | An "incorrect email or master password" error |
| 2 | First login, no 2FA | Sign in with the right password | The server may ask for the **new device code** sent by email. Enter it and the vault opens |
| 3 | Wrong device code | At the step from case 2, enter a wrong code | An error shows up and the screen **stays on the same step**; you can enter the right code |
| 4 | Items | Open each item, reveal and copy the password | The values match the web vault; the clipboard is cleared after 30 s |
| 5 | Sync | Change an item in the web vault and click sync | The change shows up |
| 6 | Lock and unlock | Lock and unlock with the password | Opens without network |
| 7 | Authenticator 2FA | Turn on the authenticator in the web vault, sign out and sign in again | The app asks for the code; a wrong code keeps the screen and the right one opens the vault |
| 8 | Email 2FA | Turn on email 2FA and click "Send code by email" | "Code sent." and the email arrives |
| 9 | Remember device | Check "Remember this device", sign in, close the app and sign in again | The second login **does not ask for a code**. The token is kept in Windows Credential Manager, as "Scout Vault" |
| 10 | Security key / Windows Hello | Register a FIDO2 key (or Windows Hello) in the web vault as 2FA and choose that method in the app | The "Use a security key or Windows Hello" button opens the Windows prompt; when you touch the key, the vault opens |
| 11 | Duo | With Duo on, click "Open Duo in the browser" and approve. The browser tries to open `bitwarden://`; cancel, copy the page address and paste it into the field | The vault opens |
| 12 | SSO (organization only) | Check "Sign in with company SSO", enter the SSO identifier and the master password | The browser opens the company login; when it finishes, the page says to go back and the vault opens |
| 13 | Fill in another app | With the vault open, open the login screen of an app or site whose **window title** has the item name or website (e.g. "Steam", "github.com"), click the username field and press `Ctrl+Alt+A` | Scout Vault types username ⇥ password ↵. If more than one item matches (or none), Scout Vault shows up with the list; after you choose, it goes back to the window and types |
| 14 | Fill with the vault locked | Lock the vault and press `Ctrl+Alt+A` in the app | Scout Vault shows up asking for the password; after unlocking, it types into the app |
| 15 | Remember session | Sign in, close the app and open it again | Opens straight to **unlock** (password only). Without internet, it unlocks and shows the vault |
| 16 | Create and edit | Create a login with `+`, then edit the password | It shows up in the web vault and on the phone; in the web vault, the item's **password history** shows the old one |
| 17 | Data from other apps | In Scout Vault, edit an item that has a **passkey** or a URI rule created in the official app | The passkey and the rule are still there in the official app |
| 18 | Trash | Move to trash, restore, move again and delete permanently | Matches the web vault at every step |
| 19 | Attachments | Create a **Note** and, in the form itself, drag a PDF and an image onto "Attachments"; save. Then drag one more file onto the item detail | The files show up in the web vault and open the same. On a free bitwarden.com account, the app says attachments require Premium. Uploading to the official cloud (Azure) **has not been tested yet** |
| 20 | Browser | Settings → Browser → turn on; load the extension; on a saved site, click the Scout Vault icon | Lists the site's logins; clicking fills username and password |
| 21 | Folders | Create a folder with the sidebar `+`, put an item in it, rename it and delete it | The folder shows up the same in the web vault; after deleting it, the item is still there, under "No folder" |
| 22 | Organization | In an organization, open a collection in the sidebar, create a login with `+` and save; then edit it and check another collection | The item shows up in the organization and in the right collections in the web vault; someone with only the other collection can now see it |
| 23 | Stay signed in | In Settings → Session, set "Ask for the master password on open" to 7 days; unlock, quit from the tray and open again. Then change the master password in the web vault and sync | Reopens already unlocked; after the password change a notice shows up and the next start asks for the new password |
| 24 | Windows Hello | With the vault open, in Settings → Session turn on "Unlock with Windows Hello" and confirm face/PIN. Lock (`Ctrl+L`) and use "Use Windows Hello". Then cancel Hello once | The first Windows Hello prompt shows up **in front of** the app; the button opens the vault; canceling explains and leaves the password as an alternative. **Not tested with real Windows Hello yet** |
| 25 | Lock with Windows and re-prompt | With the vault open, press Win+L and sign back in to Windows. Then check "Ask for the master password" on an item and try to show/copy the password | The vault is locked when you come back. On the marked item, the password only shows after confirming the master password; the official app asks too |
| 26 | Save logins | Reload the extension (chrome://extensions → ↻; it asks for permission on all sites). Sign in to a site with a login that is not in the vault; then change that site's password and sign in again | "Save this login in Scout Vault?" shows up and the item appears in the vault (and in the web vault). The second time, "Update the password for …?"; the web vault shows the old one in the history. "Not now" saves nothing |
| 27 | Quick search | In any app (e.g. Notepad or a site's login screen), press `Ctrl+Shift+Space`, type part of a login's name and press `Enter`. Then open it again and use `Ctrl+P` | The small window shows up in the middle of the screen, above the app; `Enter` closes it and types username ⇥ password into the app; `Ctrl+P` copies the password and shows "Password copied". Clicking outside or `Esc` closes it |

## If something fails
For server errors, the app shows the HTTP code and the message the server sent. Copy that text
and send it along with:
- the case number;
- whether the account is US or EU;
- which 2FA methods are on for the account.

**Do not send passwords, codes or the contents of `settings.json`.** The device identifier stored
in that file is not secret, but it is not needed either.

## Known limitations
- **SSO:** the vault opens with the **master password**, like the official CLI. Accounts that rely
  on **Key Connector** or a **trusted device** (no master password) don't open; the app explains.
  If the company requires 2FA after SSO, the app says that path is not supported yet.
- **Security key:** Windows only (uses the native `webauthn.dll` API).
- **Duo:** requires pasting the final address, because the server only redirects to the official
  app (`bitwarden://`). If the official app is installed, the browser will offer to open it: decline.
- **Captcha:** if the server asks for one, the app can't show it. Sign in once through the web
  vault and try again.
- **New item types** (bank account, driver's license, passport) show up as "Type N not supported
  yet".

## What changed for the official server
- The email code request sends the `ssoEmail2faSessionToken` that the official server returns
  together with the 2FA request. It also keeps sending the password hash, which Vaultwarden uses.
- Login sends the `Device-Identifier` header and the `devicePushToken` field.
- The app now recognizes these responses: invalid device code (`device_error`), invalid 2FA code
  (which used to show up as "incorrect password") and captcha.
- If the sync doesn't bring the user key, the app uses the key that came in the login response.
- Unknown errors show the server's message, to make diagnosis easier.
- **v2 cryptography** (EncString type 7, COSE): decrypts XChaCha20-Poly1305, XAES-256-GCM,
  AES-256-GCM and the AES-CBC-HMAC AEAD; validated against Bitwarden's public test vectors.
- **Remember device**, **security key**, **Duo** and **SSO**: see cases 9 to 12.

## What was tested here
- Against the local Vaultwarden: login, sync, authenticator 2FA with a wrong and a right code,
  "remember device" across logins and an expired token (`tests/live_2fa.rs`).
- **No real server available:** SSO, security key and Duo have tests for the local parts
  (PKCE, browser callback, token format), but the full path depends on your testing.
