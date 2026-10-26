#!/bin/sh
# Installs a prebuilt Boundrail release binary after checking its SHA-256.
#
#   curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/dogebonker/boundrail/main/install.sh | sh
#
#   BOUNDRAIL_VERSION        release to install, `0.1.0` or `v0.1.0` (default: the latest release)
#   BOUNDRAIL_INSTALL_DIR    where to put the binary (default: $HOME/.local/bin)
#   BOUNDRAIL_DOWNLOAD_BASE  release URL base; files are read from <base>/<tag>/<file>
#                            (default: https://github.com/dogebonker/boundrail/releases/download)
set -eu

repo=https://github.com/dogebonker/boundrail

die() {
  echo "boundrail install: $*" >&2
  exit 1
}

command -v curl >/dev/null 2>&1 || die "curl is required"
# HTTPS only, also across redirects, unless a custom download base (a mirror, a test server) is set.
fetch() {
  if [ -n "${BOUNDRAIL_DOWNLOAD_BASE:-}" ]; then
    curl -fsSL "$@"
  else
    curl --proto '=https' --tlsv1.2 -fsSL "$@"
  fi
}

case $(uname -s) in
  Linux) os=unknown-linux-musl ;;
  Darwin) os=apple-darwin ;;
  MINGW* | MSYS* | CYGWIN* | Windows_NT)
    die "on Windows, download the matching release archive from $repo/releases, or build from source" ;;
  *) die "no prebuilt boundrail for $(uname -s); build it with: cargo install --git $repo --locked" ;;
esac
case $(uname -m) in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) die "no prebuilt boundrail for $(uname -m); build it with: cargo install --git $repo --locked" ;;
esac
target=$arch-$os

base=${BOUNDRAIL_DOWNLOAD_BASE:-$repo/releases/download}
base=${base%/}
version=${BOUNDRAIL_VERSION:-}
if [ -z "$version" ]; then
  [ -z "${BOUNDRAIL_DOWNLOAD_BASE:-}" ] || die "set BOUNDRAIL_VERSION when BOUNDRAIL_DOWNLOAD_BASE is set"
  # The latest-release page redirects to .../releases/tag/<tag>.
  latest=$(fetch -I -o /dev/null -w '%{url_effective}' "$repo/releases/latest") ||
    die "could not resolve the latest release from $repo/releases/latest"
  version=${latest##*/}
  case $version in
    v[0-9]*) ;;
    *) die "could not resolve the latest release (got $latest); set BOUNDRAIL_VERSION" ;;
  esac
fi
tag=v${version#v}

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
  die "sha256sum or shasum is required to verify the download; not installing"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fetch -o "$tmp/SHA256SUMS" "$base/$tag/SHA256SUMS" || die "could not download $base/$tag/SHA256SUMS"

stem=boundrail-$tag-$target
archive=$stem.tar.gz
expected=$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")
[ -n "$expected" ] || die "SHA256SUMS has no entry for $archive; not installing"
echo "boundrail install: downloading $archive"
fetch -o "$tmp/$archive" "$base/$tag/$archive" || die "could not download $base/$tag/$archive"
actual=$(sha256 "$tmp/$archive")
[ "$actual" = "$expected" ] ||
  die "checksum mismatch for $archive: expected $expected, got $actual; not installing"

tar -xzf "$tmp/$archive" -C "$tmp"
[ -f "$tmp/$stem/boundrail" ] || die "$archive holds no $stem/boundrail"

dir=${BOUNDRAIL_INSTALL_DIR:-$HOME/.local/bin}
mkdir -p "$dir"
cp "$tmp/$stem/boundrail" "$dir/.boundrail.tmp"
chmod 755 "$dir/.boundrail.tmp"
mv -f "$dir/.boundrail.tmp" "$dir/boundrail"

echo "boundrail install: installed $tag to $dir/boundrail"
case :$PATH: in
  *:"$dir":*) ;;
  *) echo "boundrail install: $dir is not on PATH; add it: export PATH=\"$dir:\$PATH\"" ;;
esac
