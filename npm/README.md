# archgraph-cli

Runs [archgraph](https://github.com/dogebonker/archgraph), which turns a repository's import graph into evidence for architecture decisions, from npm.

```sh
npx archgraph-cli report . --out-dir /tmp/arch
```

The package has no dependencies and no install script. On first run, `bin/archgraph.js` downloads the archgraph release archive matching the package version for your platform over HTTPS, checks its SHA-256 against the release's `SHA256SUMS` shipped inside this package (downloaded from the release only when the package lacks it, as in a repository checkout), extracts the binary with the system `tar`, and caches it under `${XDG_CACHE_HOME:-~/.cache}/archgraph-cli/<version>/` (`%LOCALAPPDATA%\archgraph-cli\<version>\` on Windows). Later runs use the cached binary. The binary runs with your stdin, stdout and stderr, and its exit code is the command's.

Platforms: Linux x64 and arm64 (static musl builds), macOS x64 and arm64, Windows x64. Node 18 or later. Elsewhere, build archgraph with `cargo install --git https://github.com/dogebonker/archgraph --locked`.

A checksum mismatch, a missing archive or a network error stops with exit 2 and a message saying which; nothing is cached then.

Environment:

- `ARCHGRAPH_DOWNLOAD_BASE`: release URL base, read as `<base>/<tag>/<file>` (default `https://github.com/dogebonker/archgraph/releases/download`), for a mirror. Without it every URL, redirects included, must be HTTPS; with it, any URL is fetched.
- `ARCHGRAPH_BINARY`: run this archgraph executable and download nothing.

Scans need a collector (dependency-cruiser or grimp): see the [archgraph README](https://github.com/dogebonker/archgraph#try-it).

For coding agents, Archgraph includes five skills for context selection, architecture analysis, regression checks, fixes and rules. Follow the [skill installation instructions](https://github.com/dogebonker/archgraph#install-the-skills) using a source checkout or extracted release archive.

MIT OR Apache-2.0.
