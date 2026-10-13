#!/usr/bin/env node
// Sync the GitHub README into the npm package.
//
// npm renders the README from inside the tarball — it never fetches the repo's copy —
// so the two drift the moment one is edited. This regenerates npm/README.md from the
// root README and rewrites the one path npm cannot resolve.
//
// On GitHub, `./.assets/banner.png` resolves against the repo. On npmjs.com relative
// image paths are served relative to the registry and simply 404, so they are rewritten
// to the raw content URL. shields.io badges are already absolute and pass through.

'use strict';

const fs = require('fs');
const path = require('path');

const ROOT = path.join(__dirname, '..');
const SRC = path.join(ROOT, 'README.md');
const DST = path.join(ROOT, 'npm', 'README.md');
const RAW = 'https://raw.githubusercontent.com/vakandi/opencode-rust/main';
const BLOB = 'https://github.com/vakandi/opencode-rust/blob/main';

let md = fs.readFileSync(SRC, 'utf8');

// Both syntaxes appear: the banner uses an HTML <img src> for the width attribute,
// while other images use markdown ![alt](path). Both need the same rewrite.
md = md
  .replace(/(<img[^>]*?\ssrc=")(\.\/[^"]+)(")/g, (_m, a, rel, b) => `${a}${RAW}/${rel.slice(2)}${b}`);

// Relative *links* break the same way: `](./docs)` on npm resolves against the registry.
// Point them at the repo instead. Anchors (#section) are left alone.
md = md.replace(
  /\]\((\.\/[^)#]+)(#[^)]*)?\)/g,
  (_m, rel, anchor = '') => `](${BLOB}/${rel.slice(2)}${anchor})`,
);

const leftover = md.match(/!\[[^\]]*\]\(\.\/|<img[^>]*\ssrc="\.\/|\]\(\.\//g) || [];
if (leftover.length) {
  console.error(`[sync-readme] ${leftover.length} relative path(s) survived the rewrite`);
  process.exit(1);
}

fs.writeFileSync(DST, md);

console.log(
  `[sync-readme] ${path.relative(ROOT, SRC)} -> ${path.relative(ROOT, DST)} ` +
  `(${md.split('\n').length} lines)`,
);