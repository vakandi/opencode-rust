import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import http from "node:http";
import net from "node:net";
import fs from "node:fs";
import os from "node:os";

// proxy-local-omp — residential forward proxy + auto-rotate for omp.
// Port of the opencode proxy-local plugin, MINUS quarantine/canary/resume:
// on provider rate-limit errors it ONLY switches upstream natively.
// omp's own automatic-retry then replays the failed turn on the fresh IP.
//
// PER-TUI DESIGN: every omp process runs its OWN server (own port, own conf)
// and claims its OWN proxy. Rotations pick the oldest proxy not claimed by a
// live TUI — 5 TUIs rotating land on the 5 oldest, never the same one.
// No shell scripts: pool lives next to this file (./proxies.txt).
// Uses its OWN port/log files so both harnesses run side by side.

const HOME = os.homedir();
const EXT_DIR = `${HOME}/.omp/agent/extensions`;
const PROXY_CONF = `${HOME}/.proxychains.conf`;
const OWN_CONF = `${HOME}/.proxy-local-omp.${process.pid}.conf`;
const PROXY_POOL = `${EXT_DIR}/proxies.txt`;
const CLAIMS_FILE = `${HOME}/.proxy-local-omp.claims`;
const CLAIM_LOCK_DIR = `${HOME}/.proxy-local-omp.claims.lock`;
const PORT_FILE = `${HOME}/.proxy-local-omp.port`;
const LOG_FILE = `${HOME}/.proxy-local-omp.log`;
const DEBUG_FLAG = `${HOME}/.proxy-local-omp.debug`;
const DEFAULT_PORT = 18898;
const MAX_LOG_BYTES = 2 * 1024 * 1024;
const UPSTREAM_TIMEOUT_MS = 15000;
const STOP_COOLDOWN_MS = 60_000;
const TRANSIENT_COOLDOWN_MS = 300_000;
const NO_PROXY_HOSTS = new Set(["localhost", "127.0.0.1", "::1"]);

let debug = process.env.PROXY_LOCAL_OMP_DEBUG === "1";
try {
  debug = debug || fs.existsSync(DEBUG_FLAG);
} catch { /* ignore */ }
let upstream = { host: "", port: 0, user: "", pass: "" };
let upstreamRaw = "";
let server: http.Server | null = null;
let localPort = DEFAULT_PORT;
let reqId = 0;
let lastStopRotateAt = 0;
let lastTransientRotateAt = 0;
let rotateInFlight = false;
const INTERVAL_MS = 4 * 60_000;

function log(level: string, ...args: unknown[]): void {
  if (level === "debug" && !debug) return;
  try {
    if (fs.existsSync(LOG_FILE) && fs.statSync(LOG_FILE).size > MAX_LOG_BYTES) {
      fs.renameSync(LOG_FILE, `${LOG_FILE}.1`);
    }
    fs.appendFileSync(LOG_FILE, `[${new Date().toISOString()}] ${level.toUpperCase()} ${args.map(String).join(" ")}\n`);
  } catch { /* logging must never crash */ }
}

function parseConfLine(line: string): { host: string; port: number; user: string; pass: string } | null {
  const m = line.trim().match(/^http\s+(\S+)\s+(\d+)\s+(\S+)\s+(\S+)/);
  if (!m) return null;
  return { host: m[1], port: Number(m[2]), user: m[3], pass: m[4] };
}

function loadUpstream(): { host: string; port: number; user: string; pass: string } {
  try {
    const raw = fs.readFileSync(OWN_CONF, "utf8");
    if (raw !== upstreamRaw) {
      const first = raw.split("\n").map(parseConfLine).find(Boolean);
      if (first) {
        const changed = upstream.host !== "" && (first.host !== upstream.host || first.port !== upstream.port);
        upstream = first;
        upstreamRaw = raw;
        log("info", `upstream loaded ${upstream.host}:${upstream.port}`);
        if (changed) log("info", "upstream ROTATED — new connections use it");
      } else {
        log("error", "own conf has no valid 'http' line");
      }
    }
  } catch (err) {
    log("error", "cannot read own conf", String(err));
  }
  return upstream;
}

function upstreamAuthHeader(): string {
  const u = loadUpstream();
  return `Basic ${Buffer.from(`${u.user}:${u.pass}`).toString("base64")}`;
}

function isBypass(host: string): boolean {
  return NO_PROXY_HOSTS.has(host.toLowerCase().split(":")[0]);
}
// `headersSent` alone can't gate a late write: once headers are sent but the
// response already ended (or the client disconnected), writeHead/end throws
// "write after end" → uncaught → omp crash. Gate on writability instead.
function resAlive(res: http.ServerResponse): boolean {
  return !res.writableEnded && !res.destroyed;
}
function safeHead(res: http.ServerResponse, code: number, headers?: http.OutgoingHttpHeaders): void {
  try {
    if (!resAlive(res) || res.headersSent) return;
    if (headers !== undefined) res.writeHead(code, headers);
    else res.writeHead(code);
  } catch { /* late write after client disconnect — never crash */ }
}
function safeEnd(res: http.ServerResponse, body?: string): void {
  try {
    if (!resAlive(res)) return;
    if (body !== undefined) res.end(body);
    else res.end();
  } catch { /* noop */ }
}

function handlePlainRequest(clientReq: http.IncomingMessage, clientRes: http.ServerResponse): void {
  const id = ++reqId;
  let target: URL;
  try {
    target = new URL(clientReq.url ?? "", `http://${clientReq.headers.host ?? "localhost"}`);
  } catch {
    safeHead(clientRes, 400);
    safeEnd(clientRes, "bad request");
    return;
  }
  if (target.pathname === "/__omp_proxy_health") {
    safeHead(clientRes, 200, { "content-type": "application/json" });
    safeEnd(clientRes, JSON.stringify({ ok: true, upstream: `${upstream.host}:${upstream.port}`, pid: process.pid }));
    return;
  }
  if (target.pathname.startsWith("/zen/")) {
    forwardZen(target, clientReq, clientRes);
    return;
  }
  if (isBypass(target.hostname)) {
    const proxy = http.request(
      { host: target.hostname, port: Number(target.port) || 80, path: `${target.pathname}${target.search}`, method: clientReq.method, headers: clientReq.headers },
      (upRes) => {
        safeHead(clientRes, upRes.statusCode ?? 502, upRes.headers);
        upRes.pipe(clientRes);
      },
    );
    proxy.on("error", () => {
      safeHead(clientRes, 502);
      safeEnd(clientRes, "bypass failed");
    });
    clientReq.pipe(proxy);
    return;
  }
  const u = loadUpstream();
  const headers = { ...clientReq.headers, host: target.host, "proxy-authorization": upstreamAuthHeader() };
  delete (headers as Record<string, unknown>)["proxy-connection"];
  const proxy = http.request(
    { host: u.host, port: u.port, path: target.href, method: clientReq.method, headers, timeout: UPSTREAM_TIMEOUT_MS },
    (upRes) => {
      safeHead(clientRes, upRes.statusCode ?? 502, upRes.headers);
      upRes.pipe(clientRes);
    },
  );
  proxy.on("timeout", () => {
    proxy.destroy();
    safeHead(clientRes, 504);
    safeEnd(clientRes, "upstream timeout");
  });
  proxy.on("error", (err) => {
    log("error", `#${id} upstream error`, String(err));
    safeHead(clientRes, 502);
    safeEnd(clientRes, "upstream error");
  });
  clientReq.pipe(proxy);
}

function forwardZen(
  target: URL,
  clientReq: http.IncomingMessage,
  clientRes: http.ServerResponse,
): void {
  const id = ++reqId;
  const started = Date.now();
  const u = loadUpstream();
  const path = `${target.pathname}${target.search}`;
  log("debug", `#${id} ZEN ${clientReq.method} ${path} via residential ${u.host}:${u.port}`);
  const headers = { ...clientReq.headers };
  delete (headers as Record<string, unknown>)["proxy-connection"];
  delete (headers as Record<string, unknown>)["proxy-authorization"];
  // Single-settle: timeout vs error vs upstream-close race under concurrent
  // background tasks — first wins, losers are no-ops (fixes write-after-end crash).
  let settled = false;
  let upRes: http.IncomingMessage | null = null;
  const settle = (code: number, body: string): void => {
    if (settled) return;
    settled = true;
    safeHead(clientRes, code);
    safeEnd(clientRes, body);
  };
  // Client gone → kill upstream + unpipe so late upstream writes can't fire.
  clientRes.on("close", () => {
    settled = true;
    try { clientReq.unpipe(tun); } catch { /* noop */ }
    try { upRes?.unpipe(clientRes); } catch { /* noop */ }
    try { (upRes as http.IncomingMessage | null)?.destroy(); } catch { /* noop */ }
    try { tun.destroy(); } catch { /* noop */ }
  });
  const tun = http.request(
    {
      host: u.host,
      port: u.port,
      path: `https://opencode.ai${path}`,
      method: clientReq.method,
      // No inactivity timeout: inference SSE streams pause during reasoning
      // (a 15s timeout here false-fires on every long turn and rotation-storms).
      timeout: 0,
      headers: { ...headers, host: "opencode.ai", "proxy-authorization": upstreamAuthHeader() },
    },
    (res) => {
      if (settled) {
        try { res.resume(); } catch { /* noop */ }
        return;
      }
      settled = true;
      upRes = res;
      log("debug", `#${id} zen upstream ${upRes.statusCode} in ${Date.now() - started}ms`);
      const out = { ...upRes.headers };
      delete out["content-length"];
      safeHead(clientRes, upRes.statusCode ?? 502, out);
      // Upstream died mid-stream: headers already sent, just end.
      upRes.on("error", () => { safeEnd(clientRes); });
      upRes.pipe(clientRes);
    },
  );
  tun.on("timeout", () => {
    if (settled) return;
    settled = true;
    try { tun.destroy(); } catch { /* noop */ }
    log("error", `#${id} zen upstream timeout → rotating`);
    maybeRotate("zen-timeout", "transient");
    safeHead(clientRes, 504);
    safeEnd(clientRes, "zen upstream timeout");
  });
  tun.on("error", (err) => {
    if (settled) return;
    settled = true;
    log("error", `#${id} zen upstream error → rotating`, String(err));
    maybeRotate("zen-error", "transient");
    safeHead(clientRes, 502);
    safeEnd(clientRes, "zen upstream error");
  });
  tun.on("close", () => {
    // Upstream socket ended without headers or error: first-wins 502.
    if (settled) return;
    settle(502, "zen upstream error");
  });
  clientReq.pipe(tun);
}

function handleConnect(clientReq: http.IncomingMessage, clientSocket: net.Socket, head: Buffer): void {
  const target = clientReq.url ?? "";
  const [host, portStr] = target.split(":");
  const port = Number(portStr) || 443;
  if (isBypass(host)) {
    const direct = net.connect(port, host, () => {
      clientSocket.write("HTTP/1.1 200 Connection Established\r\n\r\n");
      if (head.length) direct.write(head);
      clientSocket.pipe(direct);
      direct.pipe(clientSocket);
    });
    direct.on("error", () => clientSocket.destroy());
    return;
  }
  const u = loadUpstream();
  const up = net.connect(u.port, u.host);
  let settled = false;
  const fail = (why: string) => {
    if (settled) return;
    settled = true;
    log("error", `CONNECT ${target} failed: ${why}`);
    try {
      clientSocket.write("HTTP/1.1 502 Bad Gateway\r\n\r\n");
    } catch { /* noop */ }
    clientSocket.destroy();
    up.destroy();
  };
  up.setTimeout(UPSTREAM_TIMEOUT_MS, () => fail("upstream timeout"));
  up.on("connect", () => {
    up.write(`CONNECT ${target} HTTP/1.1\r\nHost: ${target}\r\nProxy-Authorization: ${upstreamAuthHeader()}\r\n\r\n`);
  });
  let buf = Buffer.alloc(0);
  up.on("data", (chunk: Buffer) => {
    if (settled) return;
    buf = Buffer.concat([buf, chunk]);
    const headerEnd = buf.indexOf("\r\n\r\n");
    if (headerEnd === -1) return;
    const statusLine = buf.subarray(0, buf.indexOf("\r\n")).toString();
    if (!/^HTTP\/1\.[01] 200/.test(statusLine)) {
      fail(`upstream refused tunnel: ${statusLine}`);
      return;
    }
    settled = true;
    clientSocket.write("HTTP/1.1 200 Connection Established\r\n\r\n");
    const rest = buf.subarray(headerEnd + 4);
    if (rest.length) clientSocket.write(rest);
    if (head.length) up.write(head);
    clientSocket.pipe(up);
    up.pipe(clientSocket);
  });
  up.on("error", (err) => fail(String(err)));
  up.on("close", () => {
    if (!settled) fail("closed before tunnel");
  });
}

function probeHealth(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const req = http.request(
      { host: "127.0.0.1", port, path: "/__omp_proxy_health", method: "GET", timeout: 3000 },
      (res) => {
        let body = "";
        res.on("data", (d: Buffer) => (body += d.toString()));
        res.on("end", () => {
          try {
            resolve(res.statusCode === 200 && JSON.parse(body).ok === true);
          } catch {
            resolve(false);
          }
        });
      },
    );
    req.on("timeout", () => {
      req.destroy();
      resolve(false);
    });
    req.on("error", () => resolve(false));
    req.end();
  });
}

async function startServer(): Promise<number> {
  // PER-TUI: every omp process owns its server — no sharing, so each TUI
  // keeps its own proxy even when others rotate.
  for (let port = DEFAULT_PORT; port < DEFAULT_PORT + 20; port++) {
    try {
      await new Promise<void>((resolve, reject) => {
        const srv = http.createServer();
        srv.on("request", handlePlainRequest);
        srv.on("connect", handleConnect);
        srv.once("error", reject);
        srv.listen(port, "127.0.0.1", () => {
          srv.removeAllListeners("error");
          srv.on("error", (err) => log("error", `server error :${port}`, String(err)));
          server = srv;
          resolve();
        });
      });
      localPort = port;
      fs.writeFileSync(PORT_FILE, String(port));
      log("info", `listening on 127.0.0.1:${port}`);
      return port;
    } catch {
      log("debug", `port ${port} busy, trying next`);
    }
  }
  throw new Error("no free port in 18898-18917");
}

// --- rotation (switch-only, NO quarantine) ---

const NO_ROTATE_RE = /Endpoint is unavailable|Cannot connect to API|ECONNREFUSED|socket closed|MessageAbortedError|Aborted/i;
const STOP_RE = /FreeUsageLimitError|FreeTierError|free tier can only be used|Free usage exceeded|Free limit reached|subscribe to Go|GoUsageLimitError|account_rate_limit|retry-after-ms|errorStatus["']?\s*:\s*429|"429"|429 Rate limit/i;
const TRANSIENT_RE = /rate_limit_exceeded|Rate limit exceeded|service_overloaded|temporarily overloaded|Internal server error|500/i;

const RETRY_NUDGE = "Proxy rotated after a Zen rate limit. Continue exactly where you left off — retry the last failed step, do not restart the task.";
const RETRY_MAX_PER_HOUR = 3;
const retryCount = new Map<string, { count: number; resetAt: number }>();

function forceRetry(reason: string): void {
  const now = Date.now();
  const slot = retryCount.get("self");
  if (slot && now < slot.resetAt) {
    if (slot.count >= RETRY_MAX_PER_HOUR) {
      log("info", `force retry skipped (cap ${RETRY_MAX_PER_HOUR}/h reached) [${reason}]`);
      return;
    }
  } else {
    retryCount.set("self", { count: 0, resetAt: now + 3_600_000 });
  }
  try {
    const fn = (piRef as unknown as { sendUserMessage?: (c: string) => void })?.sendUserMessage;
    if (typeof fn !== "function") {
      log("error", `force retry ABORTED — sendUserMessage unavailable [${reason}]`);
      return;
    }
    fn.call(piRef, RETRY_NUDGE);
    retryCount.get("self")!.count++;
    log("info", `force retry sent [${reason}]`);
  } catch (err) {
    log("error", "force retry failed", String(err));
  }
}

let piRef: unknown = null;
type UiNotify = { notify?: (m: string, t?: string) => void };
let lastUi: { ui?: UiNotify } | null = null;
function rememberUi(ctx: unknown): void {
  try {
    const u = (ctx as { ui?: UiNotify })?.ui;
    if (u && typeof u.notify === "function") lastUi = { ui: u };
  } catch { /* noop */ }
}
// Banner for background paths (interval / auto-retry / tool). Uses the last
// seen command/event ctx — best-effort, always logs either way.
function banner(msg: string, type: string): void {
  try {
    lastUi?.ui?.notify?.(msg, type);
    log("info", `banner ${type}: ${msg} (ui ${lastUi ? "shown" : "MISSING — no command/event ctx seen yet"})`);
  } catch (err) { log("error", "banner failed", String(err)); }
}

type Verdict = "stop" | "transient" | "ignore";

function classifyProviderError(text: string): Verdict {
  if (NO_ROTATE_RE.test(text)) return "ignore";
  if (STOP_RE.test(text)) return "stop";
  if (TRANSIENT_RE.test(text)) return "transient";
  return "ignore";
}

type SwitchResult = { switched: boolean; detail: string; from: string; to: string };

type Claim = { pid: number; proxyIndex: number; claimedAt: number };

function readClaims(): Claim[] {
  try {
    const v = JSON.parse(fs.readFileSync(CLAIMS_FILE, "utf8"));
    return Array.isArray(v) ? v.filter((c) => typeof c?.pid === "number" && typeof c?.proxyIndex === "number") : [];
  } catch { return []; }
}

function writeClaims(claims: Claim[]): void {
  try { fs.writeFileSync(CLAIMS_FILE, JSON.stringify(claims)); } catch { /* best-effort */ }
}

function cleanStaleClaims(claims: Claim[]): Claim[] {
  return claims.filter((c) => {
    try { process.kill(c.pid, 0); return true; } catch { return false; }
  });
}

async function acquireClaimLock(): Promise<boolean> {
  for (let i = 0; i < 80; i++) {
    try {
      fs.mkdirSync(CLAIM_LOCK_DIR);
      try { fs.writeFileSync(`${CLAIM_LOCK_DIR}/owner`, `${process.pid}:${Date.now()}`); } catch { /* best-effort */ }
      return true;
    } catch {
      let stale = false;
      try { stale = Date.now() - fs.statSync(CLAIM_LOCK_DIR).mtimeMs > 15000; } catch { /* vanished, retry */ }
      if (stale) {
        try { fs.rmSync(CLAIM_LOCK_DIR, { recursive: true, force: true }); } catch { /* retry */ }
        continue;
      }
      await new Promise((r) => setTimeout(r, 60));
    }
  }
  return false;
}

function releaseClaimLock(): void {
  try { fs.rmSync(CLAIM_LOCK_DIR, { recursive: true, force: true }); } catch { /* best-effort */ }
}

type PoolEntry = { host: string; port: number; user: string; pass: string; last: string };

/** Parse pool lines: `ip:port:user:pass |last:YYYY-MM-DD HH:MM:SS |dur:…`. */
function parsePoolLine(line: string): (PoolEntry | null) {
  const cells = line.trim().split(" |");
  const parts = (cells[0] ?? "").trim().split(":");
  const port = Number.parseInt(parts[1] ?? "", 10);
  if (!parts[0] || !Number.isFinite(port) || port <= 0 || !parts[2] || !parts[3]) return null;
  return { host: parts[0], port, user: parts[2], pass: parts.slice(3).join(":"), last: (cells[1] ?? "").replace("last:", "").trim() };
}

/** Oldest pool entry not claimed by a live TUI. 5 TUIs → 5 oldest, never the same. */
function selectOldestUnclaimed(from: string): { index: number; entry: PoolEntry } {
  const lines = fs.readFileSync(PROXY_POOL, "utf8").split("\n");
  const pool = lines.map(parsePoolLine).filter((p): p is PoolEntry => p !== null);
  if (pool.length === 0) throw new Error("empty pool");
  const taken = new Set(cleanStaleClaims(readClaims()).map((c) => c.proxyIndex));
  const sorted = pool
    .map((entry, index) => ({ index, entry, ts: entry.last ? new Date(entry.last).getTime() : 0 }))
    .sort((a, b) => a.ts - b.ts);
  const pick = sorted.find((it) => !taken.has(it.index) && `${it.entry.host}:${it.entry.port}` !== from)
    ?? sorted.find((it) => `${it.entry.host}:${it.entry.port}` !== from)
    ?? sorted[0];
  return { index: pick.index, entry: pick.entry };
}

function runSwitch(reason: string): Promise<SwitchResult> {
  const from = `${loadUpstream().host}:${loadUpstream().port}`;
  return (async (): Promise<SwitchResult> => {
    const failed = (detail: string): SwitchResult => {
      log("info", `switch failed (${detail}) [${reason}] — still on ${from}`);
      return { switched: false, detail, from, to: from };
    };
    const locked = await acquireClaimLock();
    if (!locked) log("error", "claim lock busy after 5s — proceeding without cross-process guard");
    try {
      log("info", `rotating upstream [${reason}]`);
      const { index, entry } = selectOldestUnclaimed(from);
      const claims = cleanStaleClaims(readClaims()).filter((c) => c.pid !== process.pid);
      claims.push({ pid: process.pid, proxyIndex: index, claimedAt: Date.now() });
      writeClaims(claims);
      const now = new Date().toISOString().replace("T", " ").slice(0, 19);
      const conf = ["strict_chain", "proxy_dns", "remote_dns_subnet 224", "tcp_read_time_out 15000", "tcp_connect_time_out 8000", "", "[ProxyList]", `http ${entry.host} ${entry.port} ${entry.user} ${entry.pass}`, ""].join("\n");
      fs.writeFileSync(OWN_CONF, conf);
      try {
        const lines = fs.readFileSync(PROXY_POOL, "utf8").split("\n");
        let n = 0;
        const next = lines.map((line) => {
          const t = line.trim();
          if (!t) return line;
          const addr = (t.split(" |")[0] ?? "").trim().split(":");
          if (addr[0] === entry.host && addr[1] === String(entry.port)) { n++; return `${addr.slice(0, 4).join(":")} |last:${now} |dur:0h 0m`; }
          return line;
        });
        if (n) fs.writeFileSync(PROXY_POOL, next.join("\n"));
      } catch (err) { log("error", "pool timestamp update failed", String(err)); }
      upstream = { host: entry.host, port: entry.port, user: entry.user, pass: entry.pass };
      upstreamRaw = conf;
      const to = `${entry.host}:${entry.port}`;
      const ok = to !== from;
      log("info", `switched ${from} → ${to} (idx ${index})${ok ? "" : " (UNCHANGED)"}`);
      return { switched: ok, detail: ok ? "ok" : "pool exhausted/unchanged", from, to };
    } catch (err) {
      log("error", "native switch failed", String(err));
      return failed("native failed");
    } finally {
      if (locked) releaseClaimLock();
    }
  })();
}

const STATE_FILE = `${HOME}/.proxy-local-omp.state.json`;
const SHARED_ERROR_MS = 3 * 60_000;

function readSharedLastRotate(kind: "interval" | "reactive"): number {
  try {
    const s = JSON.parse(fs.readFileSync(STATE_FILE, "utf8"));
    const v = kind === "interval" ? s.lastRotateAt : s.lastReactiveRotateAt;
    return typeof v === "number" ? v : 0;
  } catch {
    return 0;
  }
}

function writeSharedLastRotate(now: number, kind: "interval" | "reactive"): void {
  try {
    let s: Record<string, unknown> = {};
    try {
      s = JSON.parse(fs.readFileSync(STATE_FILE, "utf8"));
    } catch { /* fresh */ }
    if (kind === "interval") s.lastRotateAt = now;
    else s.lastReactiveRotateAt = now;
    s.pid = process.pid;
    fs.writeFileSync(STATE_FILE, JSON.stringify(s));
  } catch (err) {
    log("error", "shared state write failed", String(err));
  }
}

let lastIntervalRotateAt = 0;
function maybeRotate(reason: string, tier: "stop" | "transient" | "interval"): boolean {
  const now = Date.now();
  const reactive = tier !== "interval";
  const cooldown = tier === "stop" ? STOP_COOLDOWN_MS : tier === "interval" ? INTERVAL_MS : TRANSIENT_COOLDOWN_MS;
  const last = tier === "stop" ? lastStopRotateAt : tier === "interval" ? lastIntervalRotateAt : lastTransientRotateAt;
  if (rotateInFlight) {
    log("info", `rotate skipped (in flight) [${reason}]`);
    return false;
  }
  if (reactive && now - readSharedLastRotate("reactive") < SHARED_ERROR_MS) {
    log("info", `rotate skipped (another process just rotated) [${reason}]`);
    return false;
  }
  if (now - last < cooldown) {
    log("info", `rotate skipped (cooldown) [${reason}]`);
    return false;
  }
  if (tier === "stop") lastStopRotateAt = now;
  else if (tier === "interval") lastIntervalRotateAt = now;
  else lastTransientRotateAt = now;
  if (tier !== "interval") writeSharedLastRotate(now, "reactive");
  rotateInFlight = true;
  void runSwitch(reason).then((r) => {
    if (reason === "manual-command") return; // handler already notifies
    banner(
      r.switched ? `🔄 Proxy rotated ${r.from} → ${r.to} [${reason}]` : `⚠️ Proxy NOT rotated (${r.detail}) — still on ${r.to} [${reason}]`,
      r.switched ? "info" : "warn",
    );
  }).finally(() => {
    rotateInFlight = false;
  });
  return true;
}

export default function proxyLocalOmp(pi: ExtensionAPI): void {
  piRef = pi;
  pi.setLabel?.("Proxy Local (OMP)");
  {
    const before = readClaims();
    const after = cleanStaleClaims(before);
    if (after.length !== before.length) writeClaims(after);
    try {
      for (const f of fs.readdirSync(HOME)) {
        const m = f.match(/^\.proxy-local-omp\.(\d+)\.conf$/);
        if (m && m[1] !== String(process.pid)) {
          try { process.kill(Number(m[1]), 0); } catch { try { fs.rmSync(`${HOME}/${f}`); } catch { /* best-effort */ } }
        }
      }
    } catch { /* best-effort */ }
    try { if (!fs.existsSync(OWN_CONF) && fs.existsSync(PROXY_CONF)) fs.copyFileSync(PROXY_CONF, OWN_CONF); } catch { /* best-effort */ }
  }
  loadUpstream();
  void startServer()
    .then((port) => {
      const local = `http://127.0.0.1:${port}`;
      // PI_PROXY is THE knob omp inference honors (utils/proxy.ts:
      // getProxyForProvider reads PI_PROXY_<ID> / PI_PROXY only).
      // HTTPS_PROXY is set too for child processes and other fetch paths.
      process.env.PI_PROXY = local;
      process.env.HTTP_PROXY = local;
      process.env.HTTPS_PROXY = local;
      process.env.http_proxy = local;
      process.env.https_proxy = local;
      process.env.NO_PROXY = "127.0.0.1,localhost,::1";
      process.env.no_proxy = "127.0.0.1,localhost,::1";
      log("info", `proxy env set → ${local}`);
    })
    .catch((err) => log("error", "proxy server failed to start", String(err)));

  const errText = (e: unknown): string => {
    try {
      return JSON.stringify(e).slice(0, 500);
    } catch {
      return String(e).slice(0, 500);
    }
  };

  pi.on("auto_retry_start", async (event, ctx) => {
    rememberUi(ctx);
    const verdict = classifyProviderError(errText((event as Record<string, unknown>)?.["error"] ?? event));
    if (verdict === "ignore") return;
    if (!maybeRotate(`auto_retry:${verdict}`, verdict)) return;
    setTimeout(() => forceRetry(`auto_retry:${verdict}`), 8000);
  });

  pi.on("agent_end", async (event, ctx) => {
    rememberUi(ctx);
    const ev = event as Record<string, unknown>;
    const er = ev?.["error"];
    const text = errText(er ?? ev);
    const verdict = classifyProviderError(text);
    if (verdict === "ignore") return;
    log("info", `agent_end error ${verdict} → rotating, retry follows`);
    if (!maybeRotate(`agent_end:${verdict}`, verdict)) return;
    setTimeout(() => forceRetry(`agent_end:${verdict}`), 8000);
  });

  setInterval(() => {
    if (rotateInFlight) {
      log("info", "interval rotate skipped (in flight)");
      return;
    }
    log("info", "interval tick → per-TUI 4min rotate");
    maybeRotate("interval-4min", "interval");
  }, INTERVAL_MS);

  const z = (pi as unknown as { zod: { object: (s: Record<string, unknown>) => unknown; string: () => { optional: () => unknown } } }).zod;

  pi.registerTool({
    name: "proxy_status",
    label: "Proxy Status",
    description: "Show current proxy-local-omp status: local port, upstream, debug mode",
    parameters: z.object({}) as never,
    async execute() {
      const u = loadUpstream();
      const text = JSON.stringify({ local: `127.0.0.1:${localPort}`, upstream: `${u.host}:${u.port}`, debug });
      return { content: [{ type: "text" as const, text }], details: {} };
    },
  });

  pi.registerTool({
    name: "proxy_switch",
    label: "Proxy Switch",
    description: "Rotate residential proxy now (oldest proxy not claimed by another TUI). No quarantine, switch-only.",
    parameters: z.object({}) as never,
    async execute(_id?: string, _params?: unknown, _signal?: unknown, _onUpdate?: unknown, ctx?: unknown) {
      rememberUi(ctx);
      const r = await runSwitch("manual-tool");
      banner(
        r.switched ? `🔄 Proxy rotated ${r.from} → ${r.to} [manual-tool]` : `⚠️ Proxy NOT rotated (${r.detail}) — still on ${r.to} [manual-tool]`,
        r.switched ? "info" : "warn",
      );
      const text = r.switched ? `upstream=${r.to} (rotated from ${r.from})` : `upstream=${r.to} (NOT rotated: ${r.detail})`;
      return { content: [{ type: "text" as const, text }], details: {} };
    },
  });

  pi.registerTool({
    name: "proxy_debug",
    label: "Proxy Debug",
    description: "Toggle proxy-local-omp verbose logging without restart. mode=on/off.",
    parameters: z.object({ mode: z.string() }) as never,
    async execute(_id: string, params: { mode?: string }) {
      const on = (params as { mode?: string })?.mode === "on";
      debug = on;
      try {
        if (on) fs.writeFileSync(DEBUG_FLAG, "");
        else fs.rmSync(DEBUG_FLAG, { force: true });
      } catch (err) {
        log("error", "debug flag write failed", String(err));
      }
      const text = `debug ${on ? "ON" : "OFF"}`;
      return { content: [{ type: "text" as const, text }], details: {} };
    },
  });

  pi.registerCommand("switch-proxy", {
    description: "Rotate the residential proxy now (switch-only, no quarantine)",
    handler: async (_args, ctx) => {
      rememberUi(ctx);
      const r = await runSwitch("manual-command");
      try {
        (ctx as unknown as { ui?: { notify?: (m: string, t?: string) => void } }).ui?.notify?.(
          r.switched ? `Proxy rotated ${r.from} → ${r.to}` : `Proxy NOT rotated (${r.detail}) — still on ${r.to}`,
          r.switched ? "info" : "warn",
        );
      } catch { /* notify is best-effort */ }
    },
  });

  pi.registerCommand("proxy-status", {
    description: "Show current proxy-local-omp status",
    handler: async (_args, ctx) => {
      rememberUi(ctx);
      const u = loadUpstream();
      try {
        (ctx as unknown as { ui?: { notify?: (m: string, t?: string) => void } }).ui?.notify?.(
          `Local :${localPort} upstream ${u.host}:${u.port}`,
          "info",
        );
      } catch { /* noop */ }
    },
  });
}
