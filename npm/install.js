#!/usr/bin/env node
// install.js — postinstall for opencode-rust.
//
// Resolves a native binary for the host platform and writes it to lib/resolved.json.
// It does NOT compile. Compiling at install time would force every user to have a
// ~500MB Rust toolchain and wait several minutes, and long postinstall scripts are
// routinely blocked by security scanners. The prebuilt binary is fetched from the GitHub
// release for the detected platform; building from source is an explicit
// `npm run build:from-source`, not something that happens behind someone's back.
//
// One npm package, every platform. The alternatives were tried: platform-scoped
// optionalDependencies need one npm package per target, and every extra package is a
// 404 until someone remembers to create it — which is exactly how a release shipped
// with an `npm i` that completed in one second and produced no working binary.
//
// Every step is coloured, and every failure is actionable.

'use strict';

const fs = require('fs');
const path = require('path');
const os = require('os');

// chalk is a runtime dependency of the launcher, but postinstall must not hard-fail
// if it is missing (npm hoisting, offline installs, --ignore-scripts trees).
function paint() {
  try {
    return require('chalk');
  } catch {
    const wrap = (open) => (s) => (isTTY() ? `\x1b[${open}m${s}\x1b[0m` : String(s));
    return {
      cyan: wrap(36), green: wrap(32), yellow: wrap(33),
      red: wrap(31), gray: wrap(90), bold: wrap(1),
    };
  }
}

function isTTY() {
  return process.stdout.isTTY && !process.env.NO_COLOR;
}

const c = paint();
const started = Date.now();

function step(msg) {
  console.log(`${c.cyan('›')} ${msg}`);
}
function ok(msg) {
  console.log(`${c.green('✓')} ${msg}`);
}
function warn(msg) {
  console.log(`${c.yellow('!')} ${msg}`);
}
function fail(msg) {
  console.error(`${c.red('✗')} ${msg}`);
}
function done() {
  console.log(
    `${c.green('✓')} opencode-rust installed ${c.gray(`(${((Date.now() - started) / 1000).toFixed(1)}s)`)}`,
  );
  console.log(`${c.gray('  run:')} opr --help`);
}

function binaryName() {
  return process.platform === 'win32' ? 'opencode.exe' : 'opencode';
}

// musl only exists on Linux, and it is the ABSENCE of a glibc runtime version that
// identifies it. Grepping the whole report header for the string "musl" matched
// unrelated text — it reported true on macOS — which on a glibc Linux host would have
// selected the musl asset and installed a binary that cannot run.
function isMusl() {
  if (process.platform !== 'linux') return false;
  if (process.report && process.report.getReport) {
    try {
      const header = process.report.getReport().header || {};
      if (header.glibcVersionRuntime) return false;
    } catch { /* fall through to the ldd check */ }
  }
  try {
    return fs.readFileSync('/usr/bin/ldd', 'utf8').includes('musl');
  } catch {
    return false;
  }
}

function platformSuffix() {
  const arch = process.arch === 'arm64' ? 'arm64' : 'x64';
  const prefix = 'opencode-rust-';
  if (process.platform === 'darwin') return `${prefix}darwin-${arch}`;
  if (process.platform === 'linux') {
    return `${prefix}linux-${arch}-${isMusl() ? 'musl' : 'gnu'}`;
  }
  if (process.platform === 'win32') return `${prefix}win32-x64-msvc`;
  return null;
}

// install.js sits at the package root, so __dirname is already the root — but npm may
// hoist the platform package into an ancestor's node_modules. Walking up covers both.
function nodeModulesDirs() {
  const out = [];
  let dir = __dirname;
  for (let i = 0; i < 6; i += 1) {
    out.push(path.join(dir, 'node_modules'));
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return out;
}

function candidatePaths(name) {
  const out = [];

  // install.js sits at the package root, so `__dirname` is already the root — but npm
  // hoists dependencies, and the platform package may land in any ancestor's
  // node_modules. Walk up rather than assuming one layout.
  let dir = __dirname;
  for (let i = 0; i < 6; i += 1) {
    out.push(path.join(dir, 'node_modules', platformSuffix(), 'bin', name));
    out.push(path.join(dir, 'node_modules', 'opencode-rust', 'bin', name));
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return out;
}

async function findBinary() {
  const root = path.join(__dirname, '..');
  const name = binaryName();

  // 1. Already resolved by a previous run.
  const resolvedFile = path.join(__dirname, 'lib', 'resolved.json');
  if (fs.existsSync(resolvedFile)) {
    try {
      const { path: p } = JSON.parse(fs.readFileSync(resolvedFile, 'utf8'));
      if (p && fs.existsSync(p)) return p;
    } catch { /* rewrite it below */ }
  }

  // 2. A local build someone made in place (npm run build:from-source).
  const local = path.join(root, 'bin', name);
  if (fs.existsSync(local)) return local;

  // 3. A platform binary left behind by an earlier install (npm does not always run
  //    postinstall again on a cache hit).
  if (platformSuffix()) {
    for (const p of candidatePaths(name)) {
      if (fs.existsSync(p)) return p;
    }
  }

  // 4. Download the release asset for this platform.
  return await downloadForPlatform(name);
}

// The GitHub-release asset suffix for this host. Returns null for a platform the project
// does not build, so the caller can say so instead of issuing a request that 404s.
function targetSuffix() {
  const arch = process.arch;
  if (process.platform === 'darwin') {
    if (arch === 'arm64') return 'darwin-arm64';
    if (arch === 'x64') return 'darwin-x64';
    return null;
  }
  if (process.platform === 'linux') {
    const musl = isMusl();
    if (arch === 'x64') return musl ? 'linux-x64-musl' : 'linux-x64-gnu';
    if (arch === 'arm64') return musl ? 'linux-arm64-musl' : 'linux-arm64-gnu';
    return null;
  }
  if (process.platform === 'win32') {
    return arch === 'x64' ? 'win32-x64-msvc' : null;
  }
  return null;
}

function packageVersion() {
  try {
    return require(path.join(__dirname, 'package.json')).version;
  } catch {
    return '0.0.0';
  }
}

async function downloadForPlatform(name) {
  const suffix = targetSuffix();
  if (!suffix) return null;

  const asset = `opencode-${suffix}${process.platform === 'win32' ? '.exe' : ''}`;
  const base = `https://github.com/vakandi/opencode-rust/releases/download/v${packageVersion()}`;
  const url = `${base}/${asset}`;

  const dir = path.join(__dirname, 'lib');
  fs.mkdirSync(dir, { recursive: true });
  const dest = path.join(dir, asset);

  step(`downloading ${c.gray(url)}`);

  // Fetch the checksum first, but treat its absence as non-fatal: a release published
  // before this pipeline existed has no `.sha256`, and that must not fail the install.
  let expected = null;
  try {
    const text = await fetchBinary(`${base}/${asset}.sha256`, 8000);
    expected = text.split(/\s+/)[0];
  } catch {
    warn(`no checksum published for ${asset}; continuing without one`);
  }

  try {
    await fetchBinary(url, 180000, dest);
  } catch (error) {
    fail(`download failed: ${error.message}`);
    return null;
  }

  if (expected) verifyDigest(dest, expected);
  return dest;
}

// Minimal GET with redirect following, using only Node built-ins so the postinstall has
// no dependency that could be missing or hoisted somewhere unexpected.
function fetchBinary(url, timeoutMs, dest) {
  const https = require('https');
  return new Promise((resolve, reject) => {
    const request = (target, redirects) => {
      const req = https.get(
        target,
        { headers: { 'user-agent': 'opencode-rust-installer' }, timeout: timeoutMs },
        (res) => {
          if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
            res.resume();
            if (redirects >= 5) return reject(new Error('too many redirects'));
            return request(new URL(res.headers.location, target).toString(), redirects + 1);
          }
          if (res.statusCode !== 200) {
            res.resume();
            return reject(new Error(`HTTP ${res.statusCode} for ${target}`));
          }
          if (!dest) {
            res.setEncoding('utf8');
            let body = '';
            res.on('data', (chunk) => (body += chunk));
            res.on('end', () => resolve(body));
            return;
          }
          const out = fs.createWriteStream(dest, { mode: 0o755 });
          res.pipe(out);
          out.on('finish', () => {
            try {
              fs.chmodSync(dest, 0o755);
            } catch { /* non-fatal: exec bit is best effort on odd filesystems */ }
            resolve(dest);
          });
          out.on('error', reject);
        },
      );
      req.on('timeout', () => req.destroy(new Error('timed out')));
      req.on('error', reject);
    };
    request(url, 0);
  });
}

function verifyDigest(file, expected) {
  if (!expected || !/^[0-9a-f]{64}$/i.test(expected)) return;
  const crypto = require('crypto');
  const actual = crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
  if (actual !== expected.toLowerCase()) {
    fail(`checksum mismatch for ${path.basename(file)}`);
    fail(`  expected ${expected}`);
    fail(`  actual   ${actual}`);
    process.exit(1);
  }
  ok(`checksum verified ${c.gray(expected.slice(0, 16))}…`);
}

// npm's staging flow strips `os`/`cpu` from the published manifest, which makes every
// platform package look installable everywhere. Without this, a Linux or Windows machine
// would keep a macOS binary in node_modules and could select it. Removing the mismatched
// packages is what makes "one binary per machine" true in practice rather than by request.
function pruneForeignBinaries() {
  const wanted = platformSuffix();
  if (!wanted) return [];
  if (process.env.OPR_NO_PRUNE) return [];

  const removed = [];
  for (const p of nodeModulesDirs()) {
    if (!fs.existsSync(p)) continue;
    for (const entry of fs.readdirSync(p)) {
      if (!entry.startsWith('opencode-rust-')) continue;
      if (entry === wanted || entry === 'opencode-rust') continue;
      try {
        fs.rmSync(path.join(p, entry), { recursive: true, force: true });
        removed.push(entry);
      } catch {
        /* read-only tree: harmless, the launcher still picks the right one */
      }
    }
  }
  return removed;
}

function writeResolved(binPath) {
  const dir = path.join(__dirname, 'lib');
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(
    path.join(dir, 'resolved.json'),
    JSON.stringify(
      {
        path: binPath,
        platform: process.platform,
        arch: process.arch,
        node: process.version,
        resolvedAt: new Date().toISOString(),
      },
      null,
      2,
    ) + '\n',
  );
}

async function main() {
  const target = `${process.platform}-${process.arch}${isMusl() ? '-musl' : ''}`;
  step(`opencode-rust ${c.bold('install')} ${c.gray(`on ${target} (node ${process.version}, ${os.platform()})`)}`);

  const binPath = await findBinary();

  if (!binPath) {
    warn(`no prebuilt binary available for ${c.bold(target)}`);
    console.log('');
    console.log(`  ${c.gray('The npm package installed, but no release binary could be fetched')}`);
    console.log(`  ${c.gray('for your platform. Two options:')}`);
    console.log('');
    console.log(`    ${c.cyan('1.')} brew install vakandi/tap/opencode-rust`);
    console.log(`    ${c.gray('      (macOS and Linux, builds from source on your machine)')}`);
    console.log('');
    console.log(`    ${c.gray('2.')} Build from source:`);
    console.log(`       git clone https://github.com/vakandi/opencode-rust`);
    console.log(`       cd opencode-rust && ./install.sh`);
    console.log('');
    warn(`opr will exit with an explanatory message until a binary exists for ${target}`);
    process.exit(0);
  }

  ok(`native binary ${c.gray(path.relative(path.join(__dirname, '..'), binPath))}`);

  // Fail loudly and specifically if the CPU cannot execute what we shipped.
  const { execFileSync } = require('child_process');
  try {
    // `version` is a subcommand here; there is no top-level `--version` flag.
    const out = execFileSync(binPath, ['version'], {
      encoding: 'utf8',
      timeout: 15000,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    ok(`verified ${c.gray(out.trim().split('\n')[0])}`);
  } catch (e) {
    fail(`the installed binary did not start: ${e.message}`);
    console.log(`    ${c.gray('This usually means an unsupported CPU, a quarantined file, or a missing runtimes.')}`);
    console.log(`    ${c.gray('On macOS: xattr -d com.apple.quarantine <path>')}`);
    process.exit(1);
  }

  // Make it executable when a tarball or git checkout lost the bit.
  if (process.platform !== 'win32') {
    try {
      fs.chmodSync(binPath, 0o755);
    } catch { /* best effort */ }
  }

  const pruned = pruneForeignBinaries();
  if (pruned.length) {
    ok(`pruned ${c.gray(pruned.join(', '))} ${c.gray(`(wrong platform, ${target})`)}`);
  }

  writeResolved(binPath);
  done();
}

main().catch((error) => {
  fail(error && error.stack ? error.stack : String(error));
  process.exit(1);
});