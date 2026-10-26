# boundrail-cli

This directory defines the npm wrapper for [Boundrail](https://github.com/dogebonker/boundrail).
Until the npm package is published,
[build the checked-out source](https://github.com/dogebonker/boundrail#quickstart) to use the current
CLI and features.

After publication:

```sh
npx boundrail-cli report . --engine native --out-dir /tmp/arch
```

The package has no dependencies or install script. On first run, `bin/boundrail.js` reads the
release's `SHA256SUMS` and selects the exact `boundrail-<tag>-<target>` archive. It verifies the
archive's SHA-256 before extraction and caches its executable as `boundrail`. Published packages
carry the release checksum file. A source checkout downloads it from the release.

The cache is `${XDG_CACHE_HOME:-~/.cache}/boundrail-cli/<version>/`, or
`%LOCALAPPDATA%\boundrail-cli\<version>\` on Windows. Later runs use the cached executable.
The child inherits stdin, stdout, stderr, and arguments. Its exit code is preserved.

Platforms: Linux x64 and arm64, macOS x64 and arm64, Windows x64. Node 18 or later and the system
`tar` are required. On unsupported platforms, build from source.

A checksum mismatch, missing archive, or network error exits 2 without installing that executable.

Environment overrides:

- `BOUNDRAIL_DOWNLOAD_BASE` selects a mirror as `<base>/<tag>/<file>`. Without it, every download
  URL and redirect must use HTTPS.
- `BOUNDRAIL_BINARY` selects an executable and skips downloading.

See the [main README](https://github.com/dogebonker/boundrail#quickstart)
for collector requirements and [agent workflows](https://github.com/dogebonker/boundrail#give-agents-a-project-workflow).

MIT OR Apache-2.0.
