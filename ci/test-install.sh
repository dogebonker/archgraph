#!/bin/sh
# Runs install.sh against a fake release served by `python3 -m http.server` on 127.0.0.1.
#   ci/test-install.sh <archgraph binary> <empty work dir>
# The fake release holds the given binary under every Unix target name, so install.sh finds an
# archive whatever host it detects. Checks the install, then that a wrong checksum is refused.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
binary=$1
work=$2
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$here/Cargo.toml" | head -n 1)
tag=v$version

mkdir -p "$work/release/$tag"
for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-apple-darwin aarch64-apple-darwin; do
  stem=archgraph-$tag-$target
  mkdir -p "$work/stage/$stem"
  cp "$binary" "$work/stage/$stem/archgraph"
  cp "$here/LICENSE-MIT" "$here/LICENSE-APACHE" "$here/README.md" "$work/stage/$stem/"
  tar -czf "$work/release/$tag/$stem.tar.gz" -C "$work/stage" "$stem"
done
(
  cd "$work/release/$tag"
  if command -v sha256sum >/dev/null 2>&1; then sha256sum ./*.tar.gz; else shasum -a 256 ./*.tar.gz; fi |
    sed 's| \./| |' >SHA256SUMS
)

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

ARCHGRAPH_VERSION=$version ARCHGRAPH_DOWNLOAD_BASE=http://127.0.0.1:$port \
  ARCHGRAPH_INSTALL_DIR=$work/bin sh "$here/install.sh"
"$work/bin/archgraph" --version | grep -qx "archgraph $version"
echo "ok: installed $("$work/bin/archgraph" --version)"

awk '{ printf "%064d  %s\n", 0, $2 }' "$work/release/$tag/SHA256SUMS" >"$work/sums"
mv "$work/sums" "$work/release/$tag/SHA256SUMS"
rm -f "$work/bin/archgraph"
if ARCHGRAPH_VERSION=$version ARCHGRAPH_DOWNLOAD_BASE=http://127.0.0.1:$port \
  ARCHGRAPH_INSTALL_DIR=$work/bin sh "$here/install.sh" 2>"$work/mismatch.err"; then
  echo "install.sh accepted a wrong checksum" >&2
  exit 1
fi
grep -q 'checksum mismatch' "$work/mismatch.err"
[ ! -e "$work/bin/archgraph" ]
echo "ok: refused: $(cat "$work/mismatch.err")"
