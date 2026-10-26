#!/bin/sh
# Verify the Boundrail release layout and reject unlisted archives and bad checksums.
#   ci/test-install.sh <boundrail binary> <empty work dir>
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
binary=$1
work=$2
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$here/Cargo.toml" | head -n 1)
tag=v$version
mkdir -p "$work/release/$tag"
for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-apple-darwin aarch64-apple-darwin; do
  stem=boundrail-$tag-$target
  mkdir -p "$work/stage/$stem"
  cp "$binary" "$work/stage/$stem/boundrail"
  cp "$here/LICENSE-MIT" "$here/LICENSE-APACHE" "$here/README.md" "$work/stage/$stem/"
  tar -czf "$work/release/$tag/$stem.tar.gz" -C "$work/stage" "$stem"
done
(
  cd "$work/release/$tag"
  if command -v sha256sum >/dev/null 2>&1; then sha256sum ./*.tar.gz; else shasum -a 256 ./*.tar.gz; fi |
    sed 's| \./| |' >SHA256SUMS
)
cp "$work/release/$tag/SHA256SUMS" "$work/good-sums"
python3 -u -m http.server 0 --bind 127.0.0.1 --directory "$work/release" >"$work/server.log" 2>&1 &
server=$!
trap 'kill "$server" 2>/dev/null || true' EXIT
port=
for _ in $(seq 50); do
  port=$(sed -n 's/.* port \([0-9][0-9]*\).*/\1/p' "$work/server.log" | head -n 1)
  [ -n "$port" ] && break
  sleep 0.1
done
[ -n "$port" ] || { cat "$work/server.log" >&2; exit 1; }
install_to() {
  BOUNDRAIL_VERSION=$version BOUNDRAIL_DOWNLOAD_BASE=http://127.0.0.1:$port \
    BOUNDRAIL_INSTALL_DIR=$1 ARCHGRAPH_INSTALL_DIR="$work/unsupported-old-variable" sh "$here/install.sh"
}
install_to "$work/bin" >"$work/new.log"
"$work/bin/boundrail" --version | grep -qx "boundrail $version"
grep -q 'downloading boundrail-' "$work/new.log"
[ ! -e "$work/unsupported-old-variable" ]
echo "ok: canonical archive and environment inputs install Boundrail"

awk '{ printf "%064d  %s\n", 0, $2 }' "$work/good-sums" >"$work/release/$tag/SHA256SUMS"
if install_to "$work/bad-bin" >"$work/mismatch.log" 2>"$work/mismatch.err"; then
  echo "install.sh accepted a wrong checksum" >&2; exit 1
fi
grep -q 'checksum mismatch for boundrail-' "$work/mismatch.err"
[ ! -e "$work/bad-bin/boundrail" ]
echo "ok: checksum mismatch refused"

sed 's/boundrail-/other-product-/g' "$work/good-sums" >"$work/release/$tag/SHA256SUMS"
if install_to "$work/unlisted-bin" >"$work/unlisted.log" 2>"$work/unlisted.err"; then
  echo "install.sh accepted another product archive" >&2; exit 1
fi
grep -q 'SHA256SUMS has no entry for boundrail-' "$work/unlisted.err"
[ ! -e "$work/unlisted-bin/boundrail" ]
if grep -q 'downloading' "$work/unlisted.log"; then
  echo "install.sh downloaded an unlisted archive" >&2; exit 1
fi
echo "ok: unlisted archive refused before download"
