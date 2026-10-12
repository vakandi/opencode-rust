/** Cloudflare Tunnel manager — socketless 1:1 port of live tunnel_manager.py.
 *
 * Live source (read-only): EliaAI/subworkers/server/app/services/tunnel_manager.py
 * (+ app/routes/tunnel.py for request/response shapes).
 *
 * Differences from live (intentional, wrapper-owned):
 * - Ingress service is `http://omp-server:5677` (wrapper compose service + port),
 *   not live's `http://subworker-srv:5656`.
 * - NO docker calls anywhere: `starting_cloudflared` only records a note and
 *   proceeds; the compose `cloudflared` profile + host tunnelWatch.sh own startup.
 * - `cloudflared_running` tries `docker ps` via Bun.spawnSync (5s timeout,
 *   match /cloudflared/i) but on ANY failure falls back to token-file presence.
 * - `verifying_public` is a single best-effort GET https://{domain}/health
 *   (10s timeout) → boolean, never throws; setup always lands on done afterwards.
 * - Secrets live only in memory + omp-server/config/tunnel.json /
 *   omp-server/config/tunnel.token (chmod 600); responses carry masked copies.
 */
import { existsSync, readFileSync, writeFileSync, chmodSync, unlinkSync, mkdirSync } from "node:fs";
import { join } from "node:path";

export const CF_API_BASE = "https://api.cloudflare.com/client/v4";
const CF_TIMEOUT_MS = 30_000;
const STATUS_PUBLIC_TIMEOUT_MS = 4_000;
const VERIFY_PUBLIC_TIMEOUT_MS = 10_000;

// Wrapper compose identity (container-to-container ingress target).
const COMPOSE_SERVICE_NAME = "omp-server";
const SERVER_PORT = 5677;

// Wizard progress steps exposed via status (`step` field) — exact live strings.
export const STEP_IDLE = "idle";
export const STEP_VERIFYING_TOKEN = "verifying_token";
export const STEP_CHECKING_ZONE = "checking_zone";
export const STEP_CREATING_TUNNEL = "creating_tunnel";
export const STEP_ROUTING_DNS = "routing_dns";
export const STEP_STARTING_CLOUDFLARED = "starting_cloudflared";
export const STEP_VERIFYING_PUBLIC = "verifying_public";
export const STEP_DONE = "done";
export const STEP_ERROR = "error";
export const STEPS = [
  STEP_IDLE,
  STEP_VERIFYING_TOKEN,
  STEP_CHECKING_ZONE,
  STEP_CREATING_TUNNEL,
  STEP_ROUTING_DNS,
  STEP_STARTING_CLOUDFLARED,
  STEP_VERIFYING_PUBLIC,
  STEP_DONE,
  STEP_ERROR,
] as const;

const CONFIG_DIR = new URL("./config/", import.meta.url).pathname;
const STATE_PATH = join(CONFIG_DIR, "tunnel.json");
const TOKEN_PATH = join(CONFIG_DIR, "tunnel.token");

export class TunnelError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TunnelError";
  }
}

export interface SetupOpts {
  globalKey?: string;
  email?: string;
}

// ── Module state ──────────────────────────────────────────────────────────
let step: string = STEP_IDLE;
let last_error: string | null = null;
let setupRunning = false;
let stoppedFlag = false;
let lastPublicOk = false;

export function currentStep(): string {
  return step;
}
export function lastError(): string | null {
  return last_error;
}
export function isSetupRunning(): boolean {
  return setupRunning;
}

function setStep(next: string): void {
  step = next;
}

// ── Helpers (verbatim live semantics) ─────────────────────────────────────

export function maskToken(token: string | null | undefined): string | null {
  if (!token) return null;
  if (token.length <= 6) return "…";
  return `${token.slice(0, 3)}…${token.slice(-3)}`;
}
export const mask_token = maskToken;

export function normalizeDomain(domain: string): string {
  let d = (domain ?? "").trim().toLowerCase();
  for (const prefix of ["https://", "http://"]) {
    if (d.startsWith(prefix)) d = d.slice(prefix.length);
  }
  return d.replace(/^\/+|\/+$/g, "").split("/", 1)[0] ?? "";
}
export const normalize_domain = normalizeDomain;

// ── Cloudflare API ────────────────────────────────────────────────────────

export async function cfRequest(
  method: string,
  path: string,
  apiToken: string,
  body?: Record<string, unknown>,
  params?: Record<string, unknown>,
): Promise<any> {
  const url = new URL(CF_API_BASE + path);
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined && v !== null) url.searchParams.set(k, String(v));
    }
  }
  let resp: Response;
  try {
    resp = await fetch(url.toString(), {
      method,
      headers: {
        Authorization: `Bearer ${apiToken}`,
        "Content-Type": "application/json",
      },
      body: body !== undefined ? JSON.stringify(body) : undefined,
      signal: AbortSignal.timeout(CF_TIMEOUT_MS),
    });
  } catch (exc: any) {
    throw new TunnelError(`Cloudflare API unreachable: ${exc?.message ?? String(exc)}`);
  }
  let payload: any = {};
  let text = "";
  try {
    text = await resp.text();
    payload = text ? JSON.parse(text) : {};
  } catch {
    payload = {};
  }
  if (resp.status >= 400 || !payload?.success) {
    const errors = payload?.errors?.length
      ? payload.errors
      : [{ code: resp.status, message: (text || resp.statusText || "request failed").slice(0, 200) }];
    const msg = errors.map((e: any) => `[${e?.code}] ${e?.message}`).join("; ");
    throw new TunnelError(`Cloudflare API error: ${msg}`);
  }
  return payload?.result;
}

export async function createRestrictedTokenViaGlobal(
  globalKey: string,
  email: string,
  domain: string,
): Promise<string> {
  const url = `${CF_API_BASE}/user/tokens`;
  // Permission IDs verified via /user/tokens/permission_groups (2026-08-30):
  // Zone Read + DNS Write for the zone, Tunnel Write for the account.
  const body = {
    name: `elia-auto-${domain.replace(/\./g, "-")}`,
    policies: [
      {
        effect: "allow",
        resources: { "com.cloudflare.api.account.zone.*": "*" },
        permission_groups: [
          { id: "c8fed203ed3043cba015a93ad1616f1f" },
          { id: "4755a26eedb94da69e1066d98aa820be" },
        ],
      },
      {
        effect: "allow",
        resources: { "com.cloudflare.api.account.*": "*" },
        permission_groups: [{ id: "c07321b023e944ff818fec44d8203567" }],
      },
    ],
    expires_on: "2027-12-31T00:00:00Z",
  };
  let resp: Response;
  try {
    resp = await fetch(url, {
      method: "POST",
      headers: {
        "X-Auth-Email": email,
        "X-Auth-Key": globalKey,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(CF_TIMEOUT_MS),
    });
  } catch (exc: any) {
    throw new TunnelError(`Global API unreachable: ${exc?.message ?? String(exc)}`);
  }
  let payload: any = {};
  let text = "";
  try {
    text = await resp.text();
    payload = text ? JSON.parse(text) : {};
  } catch {
    payload = {};
  }
  if (resp.status >= 400 || !payload?.success) {
    const errors = payload?.errors?.length
      ? payload.errors
      : [{ code: resp.status, message: (text || resp.statusText || "request failed").slice(0, 200) }];
    const msg = errors.map((e: any) => `[${e?.code}] ${e?.message}`).join("; ");
    throw new TunnelError(`Could not create token via Global API Key: ${msg}`);
  }
  const result = payload?.result ?? {};
  const token = result.value ?? result.id;
  if (!token) throw new TunnelError("Global API created token but no value returned");
  return String(token);
}
export const create_restricted_token_via_global = createRestrictedTokenViaGlobal;

export async function verifyToken(apiToken: string): Promise<{ account_id: string | null; account_name: string }> {
  await cfRequest("GET", "/user/tokens/verify", apiToken);
  const accounts = await cfRequest("GET", "/accounts", apiToken, undefined, { per_page: 1 });
  const list = Array.isArray(accounts) ? accounts : accounts ? [accounts] : [];
  if (list.length === 0) return { account_id: null, account_name: "" };
  const account = list[0] ?? {};
  return { account_id: account.id ?? null, account_name: account.name ?? "" };
}
export const verify_token = verifyToken;

export async function checkZone(apiToken: string, domain: string): Promise<{
  zone_id: string;
  zone_name: string;
  zone_status: string;
  account_id: string | undefined;
  account_name: string;
}> {
  // The domain may be a subdomain of the hosted zone — strip labels until one matches.
  const labels = domain.split(".");
  let zones: any[] = [];
  for (let i = 0; i < labels.length - 1; i++) {
    const candidate = labels.slice(i).join(".");
    zones = (await cfRequest("GET", "/zones", apiToken, undefined, { name: candidate })) ?? [];
    if (zones && zones.length > 0) break;
  }
  if (!zones || zones.length === 0) {
    throw new TunnelError(
      `Zone '${domain}' not found — the domain must be managed by the Cloudflare account this token belongs to`,
    );
  }
  const zone = zones[0];
  const zoneAccount = zone?.account ?? {};
  return {
    zone_id: zone.id,
    zone_name: zone?.name ?? domain,
    zone_status: zone?.status,
    account_id: zoneAccount?.id,
    account_name: zoneAccount?.name ?? "",
  };
}
export const check_zone = checkZone;

export async function createTunnel(
  apiToken: string,
  accountId: string,
  domain: string,
): Promise<{ tunnel_id: string; tunnel_token: string }> {
  let tunnelName = `elia-subworker-${domain.replace(/\./g, "-")}`;
  let result: any;
  try {
    result = await cfRequest("POST", `/accounts/${accountId}/cfd_tunnel`, apiToken, {
      name: tunnelName,
      // Remotely-managed tunnel: ingress lives in CF (configurations endpoint).
      config_src: "cloudflare",
    });
  } catch (exc: any) {
    const msg = exc?.message ?? String(exc);
    if (msg.includes("1013") && msg.includes("already have a tunnel")) {
      // Reuse the existing tunnel with that name…
      try {
        const existing = await cfRequest("GET", `/accounts/${accountId}/cfd_tunnel`, apiToken, undefined, {
          name: tunnelName,
          is_deleted: "false",
        });
        if (Array.isArray(existing) && existing.length > 0) {
          result = existing[0];
        } else if (existing && typeof existing === "object" && (existing as any).id) {
          result = existing;
        } else {
          throw new Error("not found");
        }
      } catch {
        // …else retry once with a unique suffix.
        tunnelName = `${tunnelName}-${Math.floor(Date.now() / 1000) % 10000}`;
        result = await cfRequest("POST", `/accounts/${accountId}/cfd_tunnel`, apiToken, {
          name: tunnelName,
          config_src: "cloudflare",
        });
      }
    } else {
      throw exc;
    }
  }
  const tunnelId = result.id;
  const tokenResult = await cfRequest("GET", `/accounts/${accountId}/cfd_tunnel/${tunnelId}/token`, apiToken);
  const tunnelToken = typeof tokenResult === "string" ? tokenResult : String(tokenResult);
  // Push ingress: route the public hostname to the wrapper compose service over
  // the shared Docker network. WS passthrough works automatically.
  await cfRequest("PUT", `/accounts/${accountId}/cfd_tunnel/${tunnelId}/configurations`, apiToken, {
    config: {
      ingress: [
        { hostname: domain, service: `http://${COMPOSE_SERVICE_NAME}:${SERVER_PORT}` },
        { service: "http_status:404" },
      ],
    },
  });
  return { tunnel_id: tunnelId, tunnel_token: tunnelToken };
}
export const create_tunnel = createTunnel;

export async function checkDnsRecord(apiToken: string, zoneId: string, domain: string): Promise<boolean> {
  const records = await cfRequest("GET", `/zones/${zoneId}/dns_records`, apiToken, undefined, {
    type: "CNAME",
    name: domain,
  });
  return Boolean(records && records.length > 0);
}
export const check_dns_record = checkDnsRecord;

export async function createDnsRoute(
  apiToken: string,
  zoneId: string,
  domain: string,
  tunnelId: string,
): Promise<{ record_id: string; reused: boolean }> {
  const target = `${tunnelId}.cfargotunnel.com`;
  const existing =
    (await cfRequest("GET", `/zones/${zoneId}/dns_records`, apiToken, undefined, {
      type: "CNAME",
      name: domain,
    })) ?? [];
  for (const record of existing) {
    if (record?.content === target) {
      return { record_id: record.id, reused: true };
    }
  }
  const result = await cfRequest("POST", `/zones/${zoneId}/dns_records`, apiToken, {
    type: "CNAME",
    name: domain,
    content: target,
    proxied: true,
    ttl: 1, // auto — required for proxied records
    comment: "Elia subworker server (auto-managed)",
  });
  return { record_id: result.id, reused: false };
}
export const create_dns_route = createDnsRoute;

// ── Persistence (config/tunnel.json + config/tunnel.token, chmod 600) ─────

function readToken(): string {
  try {
    if (existsSync(TOKEN_PATH)) return readFileSync(TOKEN_PATH, "utf8").trim();
  } catch {
    // no token on disk
  }
  return "";
}

function writeTokenFile(tunnelToken: string): void {
  mkdirSync(CONFIG_DIR, { recursive: true });
  writeFileSync(TOKEN_PATH, tunnelToken.trim() + "\n");
  chmodSync(TOKEN_PATH, 0o600);
}

function loadState(): Record<string, any> | null {
  try {
    if (!existsSync(STATE_PATH)) return null;
    return JSON.parse(readFileSync(STATE_PATH, "utf8"));
  } catch {
    return null;
  }
}

function saveState(fields: Record<string, unknown>): void {
  const data = loadState() ?? {};
  Object.assign(data, fields);
  const now = new Date().toISOString();
  if (!data.created_at) data.created_at = now;
  data.updated_at = now;
  mkdirSync(CONFIG_DIR, { recursive: true });
  writeFileSync(STATE_PATH, JSON.stringify(data, null, 2));
  chmodSync(STATE_PATH, 0o600);
}

// ── Probes (best-effort, never throw) ─────────────────────────────────────

async function probePublic(domain: string, timeoutMs: number): Promise<boolean> {
  try {
    const resp = await fetch(`https://${domain}/health`, { signal: AbortSignal.timeout(timeoutMs) });
    return resp.status === 200;
  } catch {
    return false;
  }
}

async function isCloudflaredRunning(): Promise<boolean> {
  let tokenPresent = false;
  try {
    tokenPresent = readToken().length > 0;
  } catch {
    tokenPresent = false;
  }
  if (!tokenPresent) return false;
  // Best-effort docker evidence; on ANY failure fall back to token-file presence
  // (Bun.spawnSync of `docker` is allowed here — it is not a socket mount).
  try {
    const b: any = (globalThis as any).Bun;
    if (b?.spawnSync) {
      const proc = b.spawnSync(["docker", "ps", "--format", "{{.Names}}\t{{.Status}}"], { timeout: 5000 });
      const out = proc?.stdout ? Buffer.from(proc.stdout).toString("utf8") : "";
      if (/cloudflared/i.test(out)) return true;
    }
  } catch {
    // fall through to token-file fallback
  }
  return tokenPresent;
}

// ── Setup orchestration ───────────────────────────────────────────────────

export async function runSetup(domain: string, apiToken: string, opts: SetupOpts = {}): Promise<void> {
  // Fire-and-forget: never throws to the caller — failures land in step=error.
  last_error = null;
  setupRunning = true;
  stoppedFlag = false;
  try {
    const normalized = normalizeDomain(domain);
    let effective = (apiToken ?? "").trim();
    const gk = (opts.globalKey ?? "").trim();
    const em = (opts.email ?? "").trim();
    if (gk) {
      if (!em || !em.includes("@")) {
        throw new TunnelError(
          "Global API Key requires your Cloudflare email. Please fill the Email field.",
        );
      }
      effective = await createRestrictedTokenViaGlobal(gk, em, normalized);
    } else if (effective.startsWith("cfk_") && em) {
      effective = await createRestrictedTokenViaGlobal(effective, em, normalized);
    }

    setStep(STEP_VERIFYING_TOKEN);
    const account = await verifyToken(effective);

    setStep(STEP_CHECKING_ZONE);
    const zone = await checkZone(effective, normalized);

    // Account resolution: /accounts listing first, zone ownership as fallback
    // (tokens scoped to Tunnel+DNS only never list accounts).
    const accountId = account.account_id ?? zone.account_id;
    if (!accountId) {
      throw new TunnelError(
        "Could not resolve the Cloudflare account for this token — add the 'Account → Cloudflare Tunnel → Edit' permission",
      );
    }

    setStep(STEP_CREATING_TUNNEL);
    const tunnel = await createTunnel(effective, accountId, normalized);

    setStep(STEP_ROUTING_DNS);
    const dns = await createDnsRoute(effective, zone.zone_id, normalized, tunnel.tunnel_id);

    writeTokenFile(tunnel.tunnel_token);
    saveState({
      domain: normalized,
      tunnel_id: tunnel.tunnel_id,
      account_id: accountId,
      zone_id: zone.zone_id,
      record_id: dns?.record_id,
      api_token: effective,
      tunnel_token: tunnel.tunnel_token,
      note: "cloudflared startup owned by compose cloudflared profile + host tunnelWatch.sh (no docker calls from wrapper)",
    });

    setStep(STEP_STARTING_CLOUDFLARED);
    // NO docker calls at all — note recorded above; compose profile owns startup.

    setStep(STEP_VERIFYING_PUBLIC);
    lastPublicOk = await probePublic(normalized, VERIFY_PUBLIC_TIMEOUT_MS);

    setStep(STEP_DONE);
  } catch (exc: any) {
    last_error = exc instanceof Error ? exc.message : String(exc);
    setStep(STEP_ERROR);
  } finally {
    setupRunning = false;
  }
}
export const run_setup = runSetup;

export function startSetup(
  domain: string,
  apiToken = "",
  opts: SetupOpts = {},
): { status: string; step: string; domain: string } {
  if (setupRunning) throw new TunnelError("A setup is already running — poll GET /tunnel/status");
  void runSetup(domain, apiToken, opts);
  return { status: "started", step, domain: normalizeDomain(domain) };
}
export const start_setup = startSetup;

/** Legacy alias: tunnelSetup(domain, token?, email?) → startSetup. */
export function tunnelSetup(
  domain: string,
  tokenOrOpts?: string | SetupOpts,
  emailOrOpts?: string | SetupOpts,
): { status: string; step: string; domain: string } {
  let apiToken = "";
  let opts: SetupOpts = {};
  if (typeof tokenOrOpts === "string") apiToken = tokenOrOpts;
  else if (tokenOrOpts) opts = { ...tokenOrOpts };
  if (typeof emailOrOpts === "string") opts = { ...opts, email: emailOrOpts };
  else if (emailOrOpts) opts = { ...opts, ...emailOrOpts };
  return startSetup(domain, apiToken, opts);
}

// ── Check (never throws) ──────────────────────────────────────────────────

export async function check(
  domain: string,
  apiToken = "",
  globalKey = "",
  email = "",
): Promise<{
  token_ok: boolean;
  account_id?: string | null;
  account_name?: string | null;
  zone_id?: string | null;
  zone_name?: string | null;
  message: string;
}> {
  try {
    const normalized = normalizeDomain(domain ?? "");
    let effective = (apiToken ?? "").trim();
    const gk = (globalKey ?? "").trim();
    const em = (email ?? "").trim();
    if (gk) {
      effective = await createRestrictedTokenViaGlobal(gk, em, normalized);
    } else if (effective.startsWith("cfk_") && em) {
      effective = await createRestrictedTokenViaGlobal(effective, em, normalized);
    }
    const account = await verifyToken(effective);
    const zone = await checkZone(effective, normalized);
    return {
      token_ok: true,
      account_id: account.account_id ?? zone.account_id ?? null,
      account_name: account.account_name || zone.account_name || "",
      zone_id: zone.zone_id,
      zone_name: zone.zone_name,
      message: "Token and zone verified — ready to create the tunnel.",
    };
  } catch (exc: any) {
    return { token_ok: false, message: exc instanceof Error ? exc.message : String(exc) };
  }
}
export const tunnelCheck = check;

// ── Status / stop / remove ────────────────────────────────────────────────

function legacySetup(state: Record<string, any>): {
  status: string;
  domain?: string;
  message?: string;
  step: string;
  started_at?: string;
} {
  const running = [
    STEP_VERIFYING_TOKEN,
    STEP_CHECKING_ZONE,
    STEP_CREATING_TUNNEL,
    STEP_ROUTING_DNS,
    STEP_STARTING_CLOUDFLARED,
    STEP_VERIFYING_PUBLIC,
  ].includes(step);
  const legacy = step === STEP_DONE ? "done" : step === STEP_ERROR ? "error" : running ? "running" : stoppedFlag ? "stopped" : "idle";
  const out: Record<string, unknown> = { status: legacy, step };
  if (state.domain) out.domain = state.domain;
  if (last_error) out.message = last_error;
  if (state.updated_at ?? state.created_at) out.started_at = state.updated_at ?? state.created_at;
  return out as { status: string; domain?: string; message?: string; step: string; started_at?: string };
}

export async function status(): Promise<Record<string, unknown>> {
  const state = loadState() ?? {};
  const domain = state.domain ?? null;
  const tunnel_id = state.tunnel_id ?? null;
  const configured = Boolean(tunnel_id && domain);
  const public_ok = configured ? await probePublic(domain, STATUS_PUBLIC_TIMEOUT_MS) : false;
  if (configured) lastPublicOk = public_ok;
  return {
    configured,
    domain,
    tunnel_id,
    cloudflared_running: await isCloudflaredRunning(),
    public_ok,
    last_error,
    step,
    api_token_masked: maskToken(state.api_token ?? null),
    tunnel_token_masked: maskToken(state.tunnel_token ?? (readToken() || null)),
    // Legacy TopBar key (kept alongside the live keys).
    setup: legacySetup(state),
  };
}
export const tunnelStatus = status;

export async function stop(): Promise<Record<string, unknown>> {
  // Live stops the container; ours keeps token+state files (sessions preserved
  // pattern) and only clears the in-memory step. tunnelWatch.sh owns shutdown.
  step = STEP_IDLE;
  stoppedFlag = true;
  return { status: "stopped" };
}
export const tunnelStop = stop;

export async function remove(): Promise<Record<string, unknown>> {
  try {
    if (existsSync(TOKEN_PATH)) unlinkSync(TOKEN_PATH);
  } catch {
    // already gone
  }
  try {
    if (existsSync(STATE_PATH)) unlinkSync(STATE_PATH);
  } catch {
    // already gone
  }
  step = STEP_IDLE;
  last_error = null;
  stoppedFlag = false;
  lastPublicOk = false;
  return { status: "removed", removed: true, remote_errors: [] };
}
export const tunnelRemove = remove;
