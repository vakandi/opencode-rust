#!/usr/bin/env node
// opencoderust — alias entry point.
//
// npm has no way to point one package name at another, so this package exists to
// carry the second spelling and delegate every invocation to the real launcher.
// Requiring it runs it: opencode-rust/bin/opr.js calls main() at load time.

'use strict';

require('opencode-rust/bin/opr.js');