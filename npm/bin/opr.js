#!/usr/bin/env node
// opr — the OpenCodeRust launcher.
//
// This is deliberately Node and not bash. The `install.sh` wrapper is a `case`
// statement over `exec`, which Windows cannot execute at all, so a bash launcher
// would have made the npm package macOS/Linux-only. Node is already a runtime
// requirement of this engine (the plugin bridge spawns Node subprocesses and MCP
// servers are launched through npx), so requiring it here adds no new dependency.
//
// Dispatch rule, unchanged from install.sh: a known subcommand is forwarded
// verbatim, anything else falls through to `tui`. `opr --some-flag` is therefore
// `opencode tui --some-flag` — that is intended, not a bug.

'use strict';

const path = require('path');
const fs = require('fs');
const { spawn } = require('child_process');

const SUBCOMMANDS = new Set([
  'serve', 'run', 'tui', 'web', 'acp', 'session', 'stats', 'db', 'config',
  'auth', 'agent', 'debug', 'mcp', 'export', 'import', 'upgrade', 'uninstall',
  'github', 'models', 'generate', 'version', 'attach', 'pr',
]);

// Where the native binary lands, per platform. `install.js` resolves the actual
// location at install time and writes the result to lib/resolved.json.
function binaryName() {
  return process.platform === 'win32' ? 'opencode.exe' : 'opencode';
}

function resolveBinary() {
  const resolvedFile = path.join(__dirname, '..', 'lib', 'resolved.json');

  if (fs.existsSync(resolvedFile)) {
    try {
      const { path: p } = JSON.parse(fs.readFileSync(resolvedFile, 'utf8'));
      if (p && fs.existsSync(p)) return p;
    } catch {
      // fall through to discovery — a corrupt file must not be fatal
    }
  }

  // Discovery: the platform-scoped optional dependency, then a local build.
  const suffix = platformSuffix();
  const scoped = path.join(
    __dirname, '..', 'node_modules', '@opencode-rust', suffix,
    'bin', binaryName(),
  );
  if (fs.existsSync(scoped)) return scoped;

  const local = path.join(
    __dirname, '..', 'node_modules', 'opencode-rust', 'bin', binaryName(),
  );
  if (fs.existsSync(local)) return local;

  return null;
}

function platformSuffix() {
  const arch = process.arch === 'arm64' ? 'arm64' : 'x64';
  if (process.platform === 'darwin') return `darwin-${arch}`;
  if (process.platform === 'linux') {
    // musl (Alpine) and gnu (glibc) ship as different packages.
    return `linux-${arch}-${isMusl() ? 'musl' : 'gnu'}`;
  }
  if (process.platform === 'win32') return 'win32-x64-msvc';
  return null;
}

function isMusl() {
  // Node on Alpine is built against musl; report it rather than guessing.
  if (process.report && process.report.getReport) {
    try {
      return /musl/i.test(JSON.stringify(process.report.getReport().header));
    } catch {
      /* fall through */
    }
  }
  try {
    return fs.readFileSync('/usr/bin/ldd', 'utf8').includes('musl');
  } catch {
    return false;
  }
}

function main() {
  const argv = process.argv.slice(2);
  const bin = resolveBinary();

  if (!bin) {
    process.stderr.write(
      '\x1b[31mopr\x1b[0m: no native binary for ' +
      `${process.platform}-${process.arch} was installed.\n` +
      'Reinstall with the matching platform package, or build from source:\n' +
      '  git clone https://github.com/vakandi/opencode-rust && ./install.sh\n',
    );
    process.exit(1);
  }

  let args = argv;
  if (argv.length === 0) {
    args = [];
  } else if (argv[0] === '-v' || argv[0] === '--version') {
    // The clap definition has no top-level `--version` flag — only a `version`
    // subcommand. Forwarding `--version` verbatim is rejected as an unknown argument.
    args = ['version'];
  } else if (!SUBCOMMANDS.has(argv[0])) {
    args = ['tui', ...argv];
  }

  const child = spawn(bin, args, { stdio: 'inherit', windowsHide: false });

  // Forward signals so Ctrl-C reaches the engine and it can drain `run`
  // properly instead of being orphaned mid-turn.
  for (const sig of ['SIGINT', 'SIGTERM']) {
    process.on(sig, () => {
      try { child.kill(sig); } catch { /* already gone */ }
    });
  }

  child.on('error', (err) => {
    process.stderr.write(`opr: failed to start ${bin}: ${err.message}\n`);
    process.exit(1);
  });

  child.on('exit', (code, signal) => {
    if (signal) process.exit(1);
    process.exit(code === null ? 1 : code);
  });
}

main();