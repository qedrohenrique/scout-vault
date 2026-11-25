#!/usr/bin/env bash
# Signs the release files with the project's Ed25519 key.
#
# Usage: RELEASE_SIGNING_KEY=<base64 seed> tools/sign_release.sh <version or tag> <folder>
#
# For every `.exe` and `.zip` in the folder it writes:
#   <file>.release.sig  Ed25519 over "scout-vault-release-v1\nfile=<name>\nversion=<version>\n
#                       sha256=<hex>\n" — the same bytes as `update::release_message` in
#                       crates/scout-vault-core/src/update.rs. The app (0.14+) only installs
#                       a file whose signature matches its name *and* the version it is
#                       offered as, so an old installer can't be passed off as a new one.
#   <file>.sig          Ed25519 over the file's bytes, for apps up to 0.13 (they only know
#                       this one). Drop it once nobody is on 0.13 any more.
#
# Only bash, coreutils and OpenSSL (3.x, already on the runner): no packages are downloaded
# into the job that holds the key. Without the key, or with a key that doesn't match the
# public key built into the app, it fails — nothing is published unsigned by accident.
set -euo pipefail

version="${1:?usage: sign_release.sh <version> <folder>}"
version="${version#v}"
folder="${2:?usage: sign_release.sh <version> <folder>}"

# Same value as `RELEASE_PUBLIC_KEY` in crates/scout-vault-core/src/update.rs.
expected="${RELEASE_PUBLIC_KEY:-xN9t1ZichP3gJX7x4NsXGSb1pbFyvUtQT7NNU6XpmcE=}"

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "not a version: $version" >&2
  exit 1
fi
if [[ -z "${RELEASE_SIGNING_KEY:-}" ]]; then
  echo "RELEASE_SIGNING_KEY is not set: refusing to publish unsigned files" >&2
  exit 1
fi

umask 077
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The secret is the raw 32-byte seed; OpenSSL wants it as PKCS#8 (fixed 16-byte prefix).
printf '%s' "$RELEASE_SIGNING_KEY" | base64 -d > "$work/seed"
if [[ "$(wc -c < "$work/seed")" -ne 32 ]]; then
  echo "RELEASE_SIGNING_KEY must be the base64 of 32 bytes" >&2
  exit 1
fi
{
  printf '\x30\x2e\x02\x01\x00\x30\x05\x06\x03\x2b\x65\x70\x04\x22\x04\x20'
  cat "$work/seed"
} > "$work/key.der"
openssl pkey -inform DER -in "$work/key.der" -out "$work/key.pem"

public="$(openssl pkey -in "$work/key.pem" -pubout -outform DER | tail -c 32 | base64 -w0)"
if [[ "$public" != "$expected" ]]; then
  echo "the signing key doesn't match the public key built into the app" >&2
  exit 1
fi

shopt -s nullglob
files=("$folder"/*.exe "$folder"/*.zip)
if [[ ${#files[@]} -eq 0 ]]; then
  echo "nothing to sign in $folder" >&2
  exit 1
fi

for path in "${files[@]}"; do
  name="$(basename "$path")"
  sha="$(sha256sum "$path" | cut -d' ' -f1)"
  printf 'scout-vault-release-v1\nfile=%s\nversion=%s\nsha256=%s\n' \
    "$name" "$version" "$sha" > "$work/message"
  openssl pkeyutl -sign -rawin -inkey "$work/key.pem" -in "$work/message" \
    | base64 -w0 > "$path.release.sig"
  openssl pkeyutl -sign -rawin -inkey "$work/key.pem" -in "$path" \
    | base64 -w0 > "$path.sig"
  echo "signed $name"
done
