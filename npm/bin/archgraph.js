#!/usr/bin/env node
// Runs the archgraph release binary for this platform, downloading and verifying it on first use.
'use strict';

const { spawnSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { version } = require('../package.json');

const REPO = 'https://github.com/dogebonker/archgraph';
const SUMS = path.join(__dirname, '..', 'SHA256SUMS');
const TARGETS = {
  'linux x64': 'x86_64-unknown-linux-musl',
  'linux arm64': 'aarch64-unknown-linux-musl',
  'darwin x64': 'x86_64-apple-darwin',
  'darwin arm64': 'aarch64-apple-darwin',
  'win32 x64': 'x86_64-pc-windows-msvc',
};

function target(platform, arch) {
  const t = TARGETS[`${platform} ${arch}`];
  if (!t) {
    throw new Error(
      `no prebuilt archgraph for ${platform} ${arch}; build it with: cargo install --git ${REPO} --locked`,
    );
  }
  return t;
}

function cacheDir(env, platform) {
  const base =
    platform === 'win32'
      ? env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local')
      : env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache');
  return path.join(base, 'archgraph-cli', version);
}

/** The hash SHA256SUMS lists for `file`. */
function expectedSum(sums, file) {
  for (const line of sums.split('\n')) {
    const [hash, name] = line.trim().split(/\s+/);
    if (name === file || name === `*${file}`) return hash.toLowerCase();
  }
  throw new Error(`SHA256SUMS has no entry for ${file}; not running it`);
}

/** The body at `url`. `strict` (the default download base) requires https on every hop. */
async function download(url, strict) {
  for (let hops = 0; ; hops++) {
    if (strict && new URL(url).protocol !== 'https:') {
      throw new Error(`refusing ${url}: not https`);
    }
    let res;
    try {
      res = await fetch(url, { redirect: strict ? 'manual' : 'follow' });
    } catch (e) {
      throw new Error(`could not download ${url}: ${(e.cause && e.cause.message) || e.message}`);
    }
    const location = res.headers.get('location');
    if (strict && res.status >= 300 && res.status < 400 && location && hops < 10) {
      if (res.body) await res.body.cancel();
      url = new URL(location, url).href;
      continue;
    }
    if (!res.ok) throw new Error(`could not download ${url}: HTTP ${res.status}`);
    return Buffer.from(await res.arrayBuffer());
  }
}

/** The cached binary's path, downloading, verifying and extracting the release on first use. */
async function ensureBinary(env = process.env, platform = process.platform, arch = process.arch) {
  const exe = platform === 'win32' ? 'archgraph.exe' : 'archgraph';
  const dir = cacheDir(env, platform);
  const bin = path.join(dir, exe);
  if (fs.existsSync(bin)) return bin;

  const t = target(platform, arch);
  const tag = `v${version}`;
  const stem = `archgraph-${tag}-${t}`;
  const archive = `${stem}${platform === 'win32' ? '.zip' : '.tar.gz'}`;
  const strict = !env.ARCHGRAPH_DOWNLOAD_BASE;
  const base = (env.ARCHGRAPH_DOWNLOAD_BASE || `${REPO}/releases/download`).replace(/\/+$/, '');
  process.stderr.write(`archgraph-cli: downloading ${archive}\n`);
  // The published package carries the release's SHA256SUMS; a checkout of the repository does not.
  const [data, sums] = await Promise.all([
    download(`${base}/${tag}/${archive}`, strict),
    fs.existsSync(SUMS) ? fs.readFileSync(SUMS) : download(`${base}/${tag}/SHA256SUMS`, strict),
  ]);
  const expected = expectedSum(sums.toString('utf8'), archive);
  const actual = createHash('sha256').update(data).digest('hex');
  if (actual !== expected) {
    throw new Error(
      `checksum mismatch for ${archive}: expected ${expected}, got ${actual}; not running it`,
    );
  }

  fs.mkdirSync(dir, { recursive: true });
  const tmp = fs.mkdtempSync(path.join(dir, '.download-'));
  try {
    fs.writeFileSync(path.join(tmp, archive), data);
    // Windows 10+ ships bsdtar, which reads zip; Git's GNU tar earlier on PATH does not.
    const tar =
      platform === 'win32' ? path.join(env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe') : 'tar';
    const r = spawnSync(tar, ['-xf', archive], { cwd: tmp, stdio: ['ignore', 'ignore', 'pipe'] });
    if (r.error || r.status !== 0) {
      throw new Error(`could not extract ${archive}: ${r.error ? r.error.message : r.stderr}`);
    }
    const extracted = path.join(tmp, stem, exe);
    fs.chmodSync(extracted, 0o755);
    // A concurrent first run may have won; rename replaces its identical file.
    fs.renameSync(extracted, bin);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  return bin;
}

async function main() {
  const bin = process.env.ARCHGRAPH_BINARY || (await ensureBinary());
  const r = spawnSync(bin, process.argv.slice(2), { stdio: 'inherit' });
  if (r.error) throw new Error(`could not run ${bin}: ${r.error.message}`);
  if (r.signal) process.kill(process.pid, r.signal);
  process.exit(r.status === null ? 1 : r.status);
}

if (require.main === module) {
  main().catch((e) => {
    process.stderr.write(`archgraph-cli: ${e.message}\n`);
    process.exit(2);
  });
}

module.exports = { target, cacheDir, expectedSum, download, ensureBinary };
