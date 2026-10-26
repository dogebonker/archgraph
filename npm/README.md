# boundrail-cli

This directory defines the npm wrapper for [Boundrail](https://github.com/dogebonker/archgraph).
The package rename is prepared for the next release. Until a Boundrail package is published,
[build the checked-out source](https://github.com/dogebonker/archgraph#quickstart) to use the current
CLI and features.

After publication:

```sh
npx boundrail-cli report . --engine native --out-dir /tmp/arch
```

The package has no dependencies or install script. On first run, `bin/boundrail.js` reads the
release's `SHA256SUMS`, prefers the exact `boundrail-<tag>-<target>` archive, and accepts the exact
legacy `archgraph-<tag>-<target>` layout only when the new entry is absent. It verifies the archive's
SHA-256 before extraction and caches its executable as `boundrail`. A checksum mismatch never
falls back to another archive. Published packages carry the release checksum file. A source
checkout downloads it from the release.

The cache is `${XDG_CACHE_HOME:-~/.cache}/boundrail-cli/<version>/`, or
`%LOCALAPPDATA%\boundrail-cli\<version>\` on Windows. Later runs use the cached executable.
The child inherits stdin, stdout, stderr, and arguments. Its exit code is preserved.

Platforms: Linux x64 and arm64, macOS x64 and arm64, Windows x64. Node 18 or later and the system
`tar` are required. On unsupported platforms, build from source.

A checksum mismatch, missing archive, or network error exits 2 without installing that executable.
Legacy archives contain the older application and may not provide current-main features.

Existing environment overrides remain supported:

- `ARCHGRAPH_DOWNLOAD_BASE` selects a mirror as `<base>/<tag>/<file>`. Without it, every download
  URL and redirect must use HTTPS.
- `ARCHGRAPH_BINARY` selects an executable and skips downloading.

The repository URL, `.archgraph/` state, report schemas, and ownership markers keep their existing
names for compatibility. See the [main README](https://github.com/dogebonker/archgraph#quickstart)
for collector requirements and [agent workflows](https://github.com/dogebonker/archgraph#give-agents-a-project-workflow).

MIT OR Apache-2.0.
