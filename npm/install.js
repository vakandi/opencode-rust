#!/usr/bin/env node
// install.js — postinstall for opencode-rust.
//
// Resolves a native binary for the host platform and writes it to lib/resolved.json.
// It does NOT compile. Compiling at install time would force every user to have a
// ~500MB Rust toolchain and wait several minutes, and long postinstall scripts are
// routinely blocked by security scanners. Prebuilt binaries come from the
// platform-scoped optionalDependencies; building from source is an explicit
// `npm run build:from-source`, not something that happens behind someone's back.
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

function isMusl() {
  if (process.report && process.report.getReport) {
    try {
      return /musl/i.test(JSON.stringify(process.report.getReport().header));
    } catch { /* fall through */ }
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

function findBinary() {
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

  // 3. The platform-scoped optional dependency.
  if (platformSuffix()) {
    for (const p of candidatePaths(name)) {
      if (fs.existsSync(p)) return p;
    }
  }

  return null;
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

function main() {
  const target = `${process.platform}-${process.arch}${isMusl() ? '-musl' : ''}`;
  step(`opencode-rust ${c.bold('install')} ${c.gray(`on ${target} (node ${process.version}, ${os.platform()})`)}`);

  const binPath = findBinary();

  if (!binPath) {
    warn(`no prebuilt binary published yet for ${c.bold(target)}`);
    console.log('');
    console.log(`  ${c.gray('This is not a failure of the install — the package is on npm,')}`);
    console.log(`  ${c.gray('but no binary has been uploaded for your platform. Two options:')}`);
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

  writeResolved(binPath);
  done();
}

main();