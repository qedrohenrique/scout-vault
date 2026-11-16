# Security

Scout Vault stores and shows passwords, so security issues are taken seriously.

## Reporting a vulnerability

**Do not open a public issue.** Use GitHub's private reporting:
[Security → Report a vulnerability](https://github.com/qedrohenrique/scout-vault/security/advisories/new).
Only the maintainers can see the report.

It helps a lot to include:
- the version (shown in Settings, or the name of the release zip);
- what the flaw allows and under which conditions (local access, another Windows user,
  malicious extension, compromised server…);
- steps to reproduce or a proof of concept.

Initial response within 7 days. Once there is a fix, it ships in a release with credit to the
reporter (if they want it) and a security advisory in the repository.

## Supported versions

Only the latest version receives fixes. The project is in test versions (0.x): see the
notes on the [releases](https://github.com/qedrohenrique/scout-vault/releases).

## In scope

- The app (`crates/`): cryptography, local storage of keys and session, communication with the
  server, Windows Hello, clipboard, autofill in apps.
- The browser extension (`extension/`) and the local channel between it and the app.
- The build and release workflows (`.github/`).

Issues in the Bitwarden server or in Vaultwarden belong to those projects:
[Bitwarden](https://bitwarden.com/help/security/) and
[Vaultwarden](https://github.com/dani-garcia/vaultwarden/security).
