#!/bin/sh
# Pinned release installer. No sudo, compiler, daemon or agent configuration.
main() {
set -eu
HB_VERSION=0.2.2
HB_BIN_DIR=${HOME:?HOME is required}/.local/bin
HB_REPLACE=no
HB_BASE=https://github.com/zhiyao/herdcats/releases/download
HB_TEMP=
HB_STAGE=
fail() { printf '%s\n' "Herdcats setup: $*" >&2; exit 1; }
cleanup() {
  if [ -n "$HB_STAGE" ]; then rm -rf "$HB_STAGE"; fi
  if [ -n "$HB_TEMP" ]; then rm -rf "$HB_TEMP"; fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) [ "$#" -ge 2 ] || fail 'Missing version'; HB_VERSION=$2; shift 2;;
    --bin-dir) [ "$#" -ge 2 ] || fail 'Missing directory'; HB_BIN_DIR=$2; shift 2;;
    --replace) HB_REPLACE=yes; shift;;
    --release-base) [ "$#" -ge 2 ] || fail 'Missing release mirror'; HB_BASE=$2; shift 2;;
    --help) printf '%s\n' 'Usage: install.sh [--version X.Y.Z] [--bin-dir PATH] [--replace] [--release-base HTTPS_URL]'; exit 0;;
    *) fail 'Unknown option';;
  esac
done
printf '%s\n' "$HB_VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || fail 'Invalid version'
case "$HB_BASE" in https://*) ;; *) fail 'Release URL must use HTTPS';; esac
case "$HB_BIN_DIR" in /*) ;; *) fail 'Install directory must be absolute';; esac
HB_DEST=$HB_BIN_DIR/herdcats-bridge
if [ -L "$HB_DEST" ] || { [ -e "$HB_DEST" ] && [ ! -f "$HB_DEST" ]; }; then fail 'Destination must be a regular file'; fi
if [ -e "$HB_DEST" ] && [ "$HB_REPLACE" != yes ]; then fail 'Already installed; use --replace to update'; fi
case "$(uname -s):$(uname -m)" in
  Darwin:arm64|Darwin:aarch64) HB_PLATFORM=darwin-arm64;;
  Darwin:x86_64) HB_PLATFORM=darwin-x86_64;;
  Linux:x86_64) HB_PLATFORM=linux-x86_64;;
  Linux:aarch64|Linux:arm64) HB_PLATFORM=linux-arm64;;
  *) fail 'Supported platforms: macOS and Linux, arm64 or x86_64';;
esac
command -v link >/dev/null 2>&1 || fail 'POSIX link utility is required'
command -v curl >/dev/null 2>&1 || fail 'curl is required'
if command -v sha256sum >/dev/null 2>&1; then HB_HASH=sha256sum
elif command -v shasum >/dev/null 2>&1; then HB_HASH=shasum
else fail 'sha256sum or shasum is required'; fi
HB_TEMP=$(mktemp -d "${TMPDIR:-/tmp}/herdcats-install.XXXXXX")
HB_ASSET=herdcats-bridge-$HB_PLATFORM
HB_URL=$HB_BASE/herdcats-bridge-v$HB_VERSION
fetch() {
  curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
    --connect-timeout 15 --max-time 120 --max-filesize 20000000 --retry 2 -o "$2" "$1" \
    || fail 'Download failed. Check the release is published and try again.'
}
fetch "$HB_URL/SHA256SUMS" "$HB_TEMP/SHA256SUMS"
fetch "$HB_URL/$HB_ASSET" "$HB_TEMP/$HB_ASSET"
HB_EXPECTED=$(awk -v name="$HB_ASSET" '$2 == name { print $1 }' "$HB_TEMP/SHA256SUMS")
[ "${#HB_EXPECTED}" -eq 64 ] || fail 'Missing or duplicate checksum'
case "$HB_EXPECTED" in *[!0-9a-f]*) fail 'Invalid checksum';; esac
if [ "$HB_HASH" = sha256sum ]; then HB_ACTUAL=$(sha256sum "$HB_TEMP/$HB_ASSET" | awk '{print $1}')
else HB_ACTUAL=$(shasum -a 256 "$HB_TEMP/$HB_ASSET" | awk '{print $1}'); fi
[ "$HB_EXPECTED" = "$HB_ACTUAL" ] || fail 'Checksum mismatch; nothing installed'
mkdir -p "$HB_BIN_DIR"
HB_STAGE=$(mktemp -d "$HB_BIN_DIR/.herdcats-bridge.XXXXXX")
cp "$HB_TEMP/$HB_ASSET" "$HB_STAGE/herdcats-bridge"
chmod 755 "$HB_STAGE/herdcats-bridge"
# Probe the verified release on the destination volume: TMPDIR may be noexec.
HB_PROBE=$("$HB_STAGE/herdcats-bridge" --version-json) || fail 'Binary cannot run on this host'
case "$HB_VERSION" in
  0.1.*) HB_CONTRACT='{"agents":["claude"],"herdr_protocol":22,"name":"herdcats-bridge","protocol":1,"version":"'"$HB_VERSION"'"}' ;;
  *) HB_CONTRACT='{"agents":["claude","codex"],"herdr_protocol":22,"name":"herdcats-bridge","protocol":1,"version":"'"$HB_VERSION"'"}' ;;
esac
[ "$HB_PROBE" = "$HB_CONTRACT" ] || fail 'Release version or protocol mismatch'
if [ "$HB_REPLACE" = yes ]; then
  if [ -L "$HB_DEST" ] || { [ -e "$HB_DEST" ] && [ ! -f "$HB_DEST" ]; }; then fail 'Destination changed; refusing replacement'; fi
  mv -f "$HB_STAGE/herdcats-bridge" "$HB_BIN_DIR/"
else
  link "$HB_STAGE/herdcats-bridge" "$HB_DEST" || fail 'Destination appeared; nothing replaced'
fi
printf '%s\n' "Installed herdcats-bridge $HB_VERSION at $HB_DEST" 'Return to Herdcats and tap Check again.'
}

# Keep invocation last so an incomplete curl | sh download cannot run setup.
main "$@"
