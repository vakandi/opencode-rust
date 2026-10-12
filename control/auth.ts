/** Shared-token auth — mirrors live app/core/auth.py exactly.
 * ELIA_AUTH_TOKEN empty/missing → auth disabled (backward compat).
 * HTTP: Authorization: Bearer <token> OR X-Elia-Token.
 * WS: ?token= OR the same two headers on the handshake.
 * GET /health stays open.
 */
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const CONFIG_DIR = new URL("./config/", import.meta.url).pathname;

export function authToken(): string {
  return (process.env.ELIA_AUTH_TOKEN ?? "").trim();
}

export function authEnabled(): boolean {
  return authToken().length > 0;
}

function suppliedFrom(authHeader: string | null, xToken: string | null, queryToken: string | null): string | null {
  const candidates = [queryToken, authHeader, xToken];
  for (let value of candidates) {
    if (!value) continue;
    if (value.toLowerCase().startsWith("bearer ")) value = value.slice(7).trim();
    const trimmed = value.trim();
    if (trimmed) return trimmed;
  }
  return null;
}

export function checkHttpAuth(req: Request, url: URL): { ok: boolean; status?: number; detail?: string } {
  if (!authEnabled()) return { ok: true };
  if (url.pathname === "/health") return { ok: true };
  // ?token= is a first-class path, like the reference: a client that cannot set
  // headers (a WS handshake) has no other way in.
  const supplied = suppliedFrom(
    req.headers.get("authorization"),
    req.headers.get("x-elia-token"),
    url.searchParams.get("token"),
  );
  if (supplied === authToken()) return { ok: true };
  return { ok: false, status: 401, detail: "Invalid or missing token" };
}

export function checkWsAuth(req: Request, url: URL): boolean {
  if (!authEnabled()) return true;
  const supplied = suppliedFrom(
    req.headers.get("authorization"),
    req.headers.get("x-elia-token") ?? req.headers.get("sec-websocket-protocol"),
    url.searchParams.get("token"),
  );
  return supplied === authToken();
}

// ── Config loading ──

export interface SubworkerEntry {
  name: string;
  enabled: boolean;
  agent_id: string;
  model?: string | null;
  variant?: string | null;
  schedule?: Record<string, unknown> | null;
  workspace?: string | null;
  timeout_minutes?: number;
  max_retries?: number;
  notify_discord?: boolean;
  [key: string]: unknown;
}

export function subworkersJsonPath(): string {
  return join(CONFIG_DIR, "subworkers.json");
}

export function loadSubworkers(): SubworkerEntry[] {
  const path = subworkersJsonPath();
  if (!existsSync(path)) return [];
  try {
    const parsed = JSON.parse(readFileSync(path, "utf8"));
    return Array.isArray(parsed?.subworkers) ? parsed.subworkers : [];
  } catch {
    return [];
  }
}

export function serverConfig(): { port: number; concurrency: number } {
  const path = join(CONFIG_DIR, "server.json");
  try {
    const parsed = JSON.parse(readFileSync(path, "utf8"));
    return {
      port: parsed.port ?? 5677,
      concurrency: parsed.concurrency ?? 8,
    };
  } catch {
    return { port: 5677, concurrency: 8 };
  }
}

export function maxConcurrentRuns(): number {
  const raw = Number.parseInt(process.env.MAX_CONCURRENT_RUNS ?? "", 10);
  if (Number.isFinite(raw) && raw > 0) return Math.min(raw, 100);
  const cfg = serverConfig();
  return Math.min(cfg.concurrency || 8, 100);
}

/** Workspace dir — `<agent>/workspace/`, like the original opencode system.
 * Registry values are repo agent homes; the session cwd is always the
 * `workspace/` subdir (created on demand), while PROMPT.md is read from the
 * agent home (parent) — see workspacePrompt in engine.ts. Probes repo homes
 * first (host checkout and container bind-mount), then the container
 * `/data/subworkers/` live mount. First PROMPT.md hit wins. */
export function effectiveWorkspace(sw: SubworkerEntry): string {
  const serverHome = join(CONFIG_DIR, "..");
  const raw = ((sw.workspace as string | null) ?? "").replace(/\/workspace\/?$/, "");
  const base = raw.split("/").filter(Boolean).pop() ?? sw.name;
  const homes = [
    raw,
    join(serverHome, "..", "subworkers", base),
    join(serverHome, "subworkers", base),
    `/data/subworkers/${base}`,
  ];
  for (const home of homes) {
    try {
      if (home && existsSync(join(home, "PROMPT.md"))) {
        const ws = join(home, "workspace");
        try {
          mkdirSync(ws, { recursive: true });
        } catch {
          // cwd creation best-effort; spawn surfaces real errors
        }
        return ws;
      }
    } catch {
      // try next candidate
    }
  }
  return join(serverHome, "..", "subworkers", sw.name, "workspace");
}
