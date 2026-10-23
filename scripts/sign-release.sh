#!/usr/bin/env sh
# Signs a release's SHA256SUMS.txt with the Glyph release key (minisign), so
# updaters trust its checksums only with a valid signature for this
# repository and tag. Usage: sign-release.sh <file> <owner/repo> <tag>
# GLYPH_RELEASE_KEY holds the secret key file's contents (repository secret).
set -eu
file="$1"; repo="$2"; tag="$3"
if [ -z "${GLYPH_RELEASE_KEY:-}" ]; then
  echo "::error::The GLYPH_RELEASE_KEY secret is not set; releases must be signed" >&2
  exit 1
fi
if ! command -v minisign >/dev/null 2>&1; then
  sudo apt-get update -qq && sudo apt-get install -y -qq minisign
fi
key="$(mktemp)"
trap 'rm -f "$key"' EXIT
printf '%s\n' "$GLYPH_RELEASE_KEY" > "$key"
comment="glyph-release repo=$repo tag=$tag"
minisign -S -s "$key" -m "$file" -t "$comment"
# The public key the updaters carry must accept it before anything is published.
pub="$(sed -n 2p "$(dirname "$0")/../release-key.pub")"
test "$(minisign -Vm "$file" -P "$pub" -Q)" = "$comment"
echo "Signed $file: $comment"
