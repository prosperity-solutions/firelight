#!/bin/sh
# The installer test (docs/HANDOVER.md §11.1): a fake release on a local web
# server, install.sh generated with its checksums, then every case:
# a good install, a tampered archive, a tampered archive with matching
# SHA256SUMS, an unknown version, no sha tool, attestation required without
# gh. POSIX sh: runs on Ubuntu, macOS and Alpine (busybox).
#
#   sh tests/installer/test.sh
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
# INSTALLER_TEST_WORK and INSTALLER_TEST_BASE_URL: for Alpine (no web
# server in busybox), the host serves $INSTALLER_TEST_WORK/www and the
# container reads it over the host network.
work=${INSTALLER_TEST_WORK:-$(mktemp -d 2>/dev/null || mktemp -d -t fl-installer)}
mkdir -p "$work"
pid=""
cleanup() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  [ -n "${INSTALLER_TEST_WORK:-}" ] || rm -rf "$work"
}
trap cleanup EXIT INT TERM

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "ok - $*"; }

if command -v sha256sum >/dev/null 2>&1; then
  sha() { sha256sum "$1" | awk '{print $1}'; }
else
  sha() { shasum -a 256 "$1" | awk '{print $1}'; }
fi

# --- A fake release: v0.9.0 and v0.9.1, every platform the same archive. ---
www="$work/www"
sums="$work/sums"
for v in v0.9.0 v0.9.1; do
  mkdir -p "$www/$v" "$sums/$v"
  for p in linux-amd64 linux-arm64 macos-amd64 macos-arm64; do
    # The real layout (scripts/ci/build-release.sh): a folder per archive.
    d="$work/build-$v/firelight-$v-$p"
    mkdir -p "$d"
    printf '#!/bin/sh\necho "firelight %s"\n' "$v" > "$d/firelight"
    chmod +x "$d/firelight"
    printf 'MIT\n' > "$d/LICENSE"
    a="firelight-$v-$p.tar.gz"
    (cd "$work/build-$v" && tar -czf "$www/$v/$a" "firelight-$v-$p")
    printf '%s  %s\n' "$(sha "$www/$v/$a")" "$a" >> "$www/$v/SHA256SUMS"
  done
  cp "$www/$v/SHA256SUMS" "$sums/$v/SHA256SUMS"
done
sh "$root/scripts/site/gen-install.sh" "$sums" "$work/install.sh" >/dev/null

# --- A local web server (python3), or the host's (INSTALLER_TEST_BASE_URL). ---
port=$(( 20000 + $$ % 10000 ))
if [ -n "${INSTALLER_TEST_BASE_URL:-}" ]; then
  :
elif command -v python3 >/dev/null 2>&1; then
  (cd "$www" && exec python3 -m http.server "$port" --bind 127.0.0.1) >/dev/null 2>&1 &
  pid=$!
else
  fail "needs python3 to serve the fake release (or INSTALLER_TEST_BASE_URL)"
fi
base=${INSTALLER_TEST_BASE_URL:-http://127.0.0.1:$port}
i=0
until { command -v curl >/dev/null 2>&1 && curl -fs "$base/v0.9.0/SHA256SUMS" >/dev/null; } || wget -q -O /dev/null "$base/v0.9.0/SHA256SUMS" 2>/dev/null; do
  i=$((i + 1)); [ "$i" -lt 50 ] || fail "the test server didn't start"; sleep 0.2
done

run() { # run <dir> [args...]: install into <dir>; output in $work/out
  d=$1; shift
  # Hermetic: a logged-in gh on this machine must not verify fake archives.
  env -u GH_TOKEN -u GITHUB_TOKEN GH_CONFIG_DIR="$work/no-gh" \
    FIRELIGHT_INSTALL_BASE_URL="$base" FIRELIGHT_INSTALL_DIR="$d" HOME="$work" \
    sh "$work/install.sh" "$@" > "$work/out" 2>&1
}

# 1. A good install of the newest version, and of a pinned one.
run "$work/bin1" || { cat "$work/out"; fail "good install"; }
[ "$("$work/bin1/firelight")" = "firelight v0.9.1" ] || fail "the newest release is the default"
run "$work/bin2" --version v0.9.0 || { cat "$work/out"; fail "pinned install"; }
[ "$("$work/bin2/firelight")" = "firelight v0.9.0" ] || fail "--version picks that release"
pass "good install (latest and --version)"

# 2. An unknown version fails and installs nothing.
if run "$work/bin3" --version v9.9.9; then fail "unknown version installed"; fi
grep -q "unknown version v9.9.9" "$work/out" || { cat "$work/out"; fail "unknown version message"; }
[ ! -e "$work/bin3/firelight" ] || fail "unknown version left a binary"
pass "unknown version refused"

# 3. A tampered archive fails with both hashes and installs nothing.
cp -R "$www/v0.9.1" "$work/v0.9.1.orig"
for f in "$www"/v0.9.1/*.tar.gz; do printf 'x' >> "$f"; done
if run "$work/bin4"; then fail "tampered archive installed"; fi
if ! grep -q "checksum mismatch" "$work/out" || ! grep -q "expected" "$work/out"; then cat "$work/out"; fail "mismatch message"; fi
[ ! -e "$work/bin4/firelight" ] || fail "tampered archive left a binary"
pass "tampered archive refused"

# 4. Tampered archive *and* SHA256SUMS on the server: the baked table wins.
for f in "$www"/v0.9.1/*.tar.gz; do
  n=$(basename "$f")
  printf '%s  %s\n' "$(sha "$f")" "$n"
done > "$www/v0.9.1/SHA256SUMS"
if run "$work/bin5"; then fail "tampered archive with matching SHA256SUMS installed"; fi
[ ! -e "$work/bin5/firelight" ] || fail "tampered SHA256SUMS left a binary"
pass "tampered SHA256SUMS can't help"
rm -rf "$www/v0.9.1" && mv "$work/v0.9.1.orig" "$www/v0.9.1"

# 5. No sha256sum and no shasum: a clear refusal.
mkdir -p "$work/nosha"
for t in sh awk tar mktemp uname rm mkdir chmod mv cat sed tr uniq gzip curl wget dirname; do
  p=$(command -v "$t" 2>/dev/null) && ln -s "$p" "$work/nosha/$t" 2>/dev/null || true
done
if FIRELIGHT_INSTALL_BASE_URL=$base FIRELIGHT_INSTALL_DIR="$work/bin6" HOME=$work PATH="$work/nosha" \
  sh "$work/install.sh" > "$work/out" 2>&1; then fail "installed without a sha tool"; fi
grep -q "needs sha256sum or shasum" "$work/out" || { cat "$work/out"; fail "missing sha tool message"; }
pass "no sha tool refused"

# 6. Attestation required, but no gh: refused.
mkdir -p "$work/nogh"
for t in sh awk tar mktemp uname rm mkdir chmod mv cat sed tr uniq gzip curl wget sha256sum shasum perl dirname; do
  p=$(command -v "$t" 2>/dev/null) && ln -s "$p" "$work/nogh/$t" 2>/dev/null || true
done
if FIRELIGHT_INSTALL_REQUIRE_ATTESTATION=1 FIRELIGHT_INSTALL_BASE_URL=$base FIRELIGHT_INSTALL_DIR="$work/bin7" HOME=$work PATH="$work/nogh" \
  sh "$work/install.sh" > "$work/out" 2>&1; then fail "installed without the required attestation"; fi
grep -q "REQUIRE_ATTESTATION" "$work/out" || { cat "$work/out"; fail "attestation message"; }
[ ! -e "$work/bin7/firelight" ] || fail "attestation refusal left a binary"
pass "required attestation without gh refused"

echo "installer: all cases pass"
