#!/bin/sh
# Verify current and legacy release layouts against a local fake release server.
#   ci/test-install.sh <boundrail binary> <empty work dir>
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
binary=$1
work=$2
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$here/Cargo.toml" | head -n 1)
tag=v$version
mkdir -p "$work/release/$tag"
cat >"$work/legacy-program" <<LEGACY
#!/bin/sh
echo "archgraph $version"
LEGACY
chmod 755 "$work/legacy-program"
make_release() {
  product=$1
  for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-apple-darwin aarch64-apple-darwin; do
    stem=$product-$tag-$target
    mkdir -p "$work/stage/$stem"
    if [ "$product" = archgraph ]; then
      cp "$work/legacy-program" "$work/stage/$stem/$product"
    else
      cp "$binary" "$work/stage/$stem/$product"
    fi
    cp "$here/LICENSE-MIT" "$here/LICENSE-APACHE" "$here/README.md" "$work/stage/$stem/"
    tar -czf "$work/release/$tag/$stem.tar.gz" -C "$work/stage" "$stem"
  done
}
make_release boundrail
make_release archgraph
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
  ARCHGRAPH_VERSION=$version ARCHGRAPH_DOWNLOAD_BASE=http://127.0.0.1:$port \
    ARCHGRAPH_INSTALL_DIR=$1 sh "$here/install.sh"
}
install_to "$work/bin" >"$work/new.log"
"$work/bin/boundrail" --version | grep -qx "boundrail $version"
grep -q 'downloading boundrail-' "$work/new.log"
echo "ok: preferred Boundrail archive and installed $("$work/bin/boundrail" --version)"

# A valid legacy archive must not be used when the preferred archive fails verification.
awk '{ if ($2 ~ /^boundrail-/) printf "%064d  %s\n", 0, $2; else print }' "$work/good-sums" >"$work/release/$tag/SHA256SUMS"
if install_to "$work/bad-bin" >"$work/mixed.log" 2>"$work/mismatch.err"; then
  echo "install.sh accepted a wrong preferred checksum" >&2; exit 1
fi
grep -q 'checksum mismatch for boundrail-' "$work/mismatch.err"
[ ! -e "$work/bad-bin/boundrail" ]
if grep -q 'downloading archgraph-' "$work/mixed.log"; then
  echo "install.sh fell back after a preferred checksum failure" >&2
  exit 1
fi
echo "ok: checksum failure did not fall back to legacy"

awk '$2 ~ /^archgraph-/ { print }' "$work/good-sums" >"$work/release/$tag/SHA256SUMS"
install_to "$work/legacy-bin" >"$work/legacy.log"
"$work/legacy-bin/boundrail" --version | grep -qx "archgraph $version"
grep -q 'downloading archgraph-' "$work/legacy.log"
echo "ok: legacy archive installed as boundrail"
awk '{ printf "%064d  %s\n", 0, $2 }' "$work/release/$tag/SHA256SUMS" >"$work/sums"
mv "$work/sums" "$work/release/$tag/SHA256SUMS"
if install_to "$work/bad-legacy-bin" 2>"$work/legacy-mismatch.err"; then
  echo "install.sh accepted a wrong legacy checksum" >&2; exit 1
fi
grep -q 'checksum mismatch for archgraph-' "$work/legacy-mismatch.err"
[ ! -e "$work/bad-legacy-bin/boundrail" ]
echo "ok: wrong legacy checksum refused"
