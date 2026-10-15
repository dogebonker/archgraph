'use strict';

const assert = require('node:assert/strict');
const { execFileSync, spawn } = require('node:child_process');
const { createHash } = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const { target, cacheDir, expectedSum, download } = require('../bin/archgraph.js');
const { version } = require('../package.json');

const WRAPPER = path.join(__dirname, '..', 'bin', 'archgraph.js');
const TAG = `v${version}`;
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), 'archgraph-cli-test-'));
test.after(() => fs.rmSync(TMP, { recursive: true, force: true }));
const tmp = (name) => fs.mkdtempSync(path.join(TMP, `${name}-`));

/** The archgraph binary built by cargo: ARCHGRAPH_TEST_BINARY, else target/release, else target/debug. */
function builtBinary() {
  const root = path.join(__dirname, '..', '..', 'target');
  const found = [
    process.env.ARCHGRAPH_TEST_BINARY,
    path.join(root, 'release', 'archgraph'),
    path.join(root, 'debug', 'archgraph'),
  ].find((p) => p && fs.existsSync(p));
  assert.ok(found, 'build archgraph first (cargo build), or set ARCHGRAPH_TEST_BINARY');
  return found;
}

/** A release directory `<dir>/<tag>/` with this host's archive and SHA256SUMS. */
function fakeRelease(binary, sumsFor = (hash) => hash) {
  const dir = tmp('release');
  const stem = `archgraph-${TAG}-${target(process.platform, process.arch)}`;
  fs.mkdirSync(path.join(dir, 'stage', stem), { recursive: true });
  fs.copyFileSync(binary, path.join(dir, 'stage', stem, 'archgraph'));
  fs.mkdirSync(path.join(dir, TAG));
  const archive = path.join(dir, TAG, `${stem}.tar.gz`);
  execFileSync('tar', ['-czf', archive, '-C', path.join(dir, 'stage'), stem]);
  const hash = createHash('sha256').update(fs.readFileSync(archive)).digest('hex');
  fs.writeFileSync(path.join(dir, TAG, 'SHA256SUMS'), `${sumsFor(hash)}  ${stem}.tar.gz\n`);
  return dir;
}

/** Serves `dir` on 127.0.0.1; `requests` lists the paths asked for. */
async function serve(dir) {
  const requests = [];
  const server = http.createServer((req, res) => {
    requests.push(req.url);
    const file = path.join(dir, path.normalize(req.url));
    if (!file.startsWith(dir) || !fs.existsSync(file) || !fs.statSync(file).isFile()) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200).end(fs.readFileSync(file));
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return { base: `http://127.0.0.1:${server.address().port}`, requests, server };
}

/** A copy of the package whose published SHA256SUMS is `sums`, as the npm release job ships it. */
function packageWithSums(sums) {
  const dir = tmp('package');
  fs.mkdirSync(path.join(dir, 'bin'));
  fs.copyFileSync(WRAPPER, path.join(dir, 'bin', 'archgraph.js'));
  fs.copyFileSync(path.join(__dirname, '..', 'package.json'), path.join(dir, 'package.json'));
  fs.writeFileSync(path.join(dir, 'SHA256SUMS'), sums);
  return path.join(dir, 'bin', 'archgraph.js');
}

/** Runs the wrapper with `env`; resolves to { status, stdout, stderr }. */
function run(args, env, wrapper = WRAPPER) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [wrapper, ...args], {
      env: { ...process.env, ARCHGRAPH_BINARY: '', ...env },
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (d) => (stdout += d));
    child.stderr.on('data', (d) => (stderr += d));
    child.on('close', (status) => resolve({ status, stdout, stderr }));
  });
}

test('maps every released platform to its target', () => {
  assert.equal(target('linux', 'x64'), 'x86_64-unknown-linux-musl');
  assert.equal(target('linux', 'arm64'), 'aarch64-unknown-linux-musl');
  assert.equal(target('darwin', 'x64'), 'x86_64-apple-darwin');
  assert.equal(target('darwin', 'arm64'), 'aarch64-apple-darwin');
  assert.equal(target('win32', 'x64'), 'x86_64-pc-windows-msvc');
  assert.throws(() => target('win32', 'arm64'), /no prebuilt archgraph for win32 arm64/);
  assert.throws(() => target('freebsd', 'x64'), /no prebuilt archgraph for freebsd x64/);
});

test('caches under XDG_CACHE_HOME, or LOCALAPPDATA on Windows', () => {
  assert.equal(cacheDir({ XDG_CACHE_HOME: '/c' }, 'linux'), path.join('/c', 'archgraph-cli', version));
  assert.equal(cacheDir({ LOCALAPPDATA: '/l' }, 'win32'), path.join('/l', 'archgraph-cli', version));
  assert.equal(cacheDir({}, 'darwin'), path.join(os.homedir(), '.cache', 'archgraph-cli', version));
});

test('reads SHA256SUMS in text and binary mode', () => {
  const sums = 'aa  a.tar.gz\nBB *b.zip\n';
  assert.equal(expectedSum(sums, 'a.tar.gz'), 'aa');
  assert.equal(expectedSum(sums, 'b.zip'), 'bb');
  assert.throws(() => expectedSum(sums, 'c.tar.gz'), /no entry for c.tar.gz/);
});

test('refuses an archive whose checksum does not match', async () => {
  const release = fakeRelease(builtBinary(), () => '0'.repeat(64));
  const cache = tmp('cache');
  const { base, server } = await serve(release);
  try {
    const r = await run(['--version'], { XDG_CACHE_HOME: cache, ARCHGRAPH_DOWNLOAD_BASE: base });
    assert.equal(r.status, 2);
    assert.match(r.stderr, /checksum mismatch for archgraph-v.*\.tar\.gz: expected 0{64}, got [0-9a-f]{64}; not running it/);
    assert.equal(fs.existsSync(path.join(cache, 'archgraph-cli', version, 'archgraph')), false);
  } finally {
    server.close();
  }
});

test('reports a missing release and a refused connection', async () => {
  const { base, server } = await serve(tmp('empty'));
  try {
    const r = await run(['--version'], { XDG_CACHE_HOME: tmp('cache'), ARCHGRAPH_DOWNLOAD_BASE: base });
    assert.equal(r.status, 2);
    assert.match(r.stderr, /could not download http:\/\/127\.0\.0\.1:\d+\/v.*: HTTP 404/);
  } finally {
    server.close();
  }
  // The server is closed now: nothing listens on its port.
  const r = await run(['--version'], { XDG_CACHE_HOME: tmp('cache'), ARCHGRAPH_DOWNLOAD_BASE: base });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /could not download http:\/\/127\.0\.0\.1:\d+\/v.*: .*ECONNREFUSED/);
});

test('downloads, verifies, extracts and runs the binary, then reuses the cache', async () => {
  const binary = builtBinary();
  const release = fakeRelease(binary);
  const cache = tmp('cache');
  const { base, requests, server } = await serve(release);
  const env = { XDG_CACHE_HOME: cache, ARCHGRAPH_DOWNLOAD_BASE: base };
  try {
    const first = await run(['--version'], env);
    assert.equal(first.status, 0, first.stderr);
    assert.equal(first.stdout.trim(), `archgraph ${version}`);
    assert.equal(requests.length, 2);
    assert.ok(fs.existsSync(path.join(cache, 'archgraph-cli', version, 'archgraph')));

    // The binary's own exit code comes through: 2 for a missing input file.
    const second = await run(['check', path.join(cache, 'missing.json')], env);
    assert.equal(second.status, 2);
    assert.match(second.stderr, /missing\.json/);
    assert.equal(requests.length, 2, 'the second run downloads nothing');
  } finally {
    server.close();
  }
});

test('ARCHGRAPH_BINARY runs that binary without downloading', async () => {
  const r = await run(['--version'], {
    ARCHGRAPH_BINARY: builtBinary(),
    XDG_CACHE_HOME: tmp('cache'),
    ARCHGRAPH_DOWNLOAD_BASE: 'http://127.0.0.1:9',
  });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stdout.trim(), `archgraph ${version}`);
});

test('without its own SHA256SUMS the package downloads the release one', () => {
  // The repository's npm/ never holds SHA256SUMS: only the release job puts it there.
  assert.equal(fs.existsSync(path.join(__dirname, '..', 'SHA256SUMS')), false);
});

test('a published SHA256SUMS is used instead of the release one', async () => {
  const release = fakeRelease(builtBinary());
  const sumsPath = path.join(release, TAG, 'SHA256SUMS');
  const good = fs.readFileSync(sumsPath, 'utf8');
  const { base, requests, server } = await serve(release);
  try {
    // The embedded sums match; the server's are wrong and never read.
    fs.writeFileSync(sumsPath, good.replace(/^[0-9a-f]{64}/, '0'.repeat(64)));
    const ok = await run(['--version'], { XDG_CACHE_HOME: tmp('cache'), ARCHGRAPH_DOWNLOAD_BASE: base }, packageWithSums(good));
    assert.equal(ok.status, 0, ok.stderr);
    assert.equal(ok.stdout.trim(), `archgraph ${version}`);
    assert.deepEqual(requests.filter((r) => r.endsWith('SHA256SUMS')), []);

    // The embedded sums are wrong; the server's match and are not consulted.
    fs.writeFileSync(sumsPath, good);
    const bad = await run(
      ['--version'],
      { XDG_CACHE_HOME: tmp('cache'), ARCHGRAPH_DOWNLOAD_BASE: base },
      packageWithSums(good.replace(/^[0-9a-f]{64}/, '0'.repeat(64))),
    );
    assert.equal(bad.status, 2);
    assert.match(bad.stderr, /checksum mismatch/);
    assert.deepEqual(requests.filter((r) => r.endsWith('SHA256SUMS')), []);
  } finally {
    server.close();
  }
});

test('downloads are https-only unless ARCHGRAPH_DOWNLOAD_BASE is set', async () => {
  // Strict (the default base): every URL, the first and each redirect hop, must be https.
  await assert.rejects(download('http://127.0.0.1:9/x', true), /refusing http:\/\/127\.0\.0\.1:9\/x: not https/);
  // With a custom base, plain http and redirects are followed.
  const server = http.createServer((req, res) => {
    if (req.url === '/first') res.writeHead(302, { location: '/next' }).end();
    else res.writeHead(200).end('payload');
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    const body = await download(`http://127.0.0.1:${server.address().port}/first`, false);
    assert.equal(body.toString(), 'payload');
  } finally {
    server.close();
  }
});
