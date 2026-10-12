/** Control-plane identity — the answer to "which server just talked to me?".
 *
 * Before this, `GET /health` returned `{status, uptime_s}` and `GET /server/health`
 * returned a hardcoded `health_status: "healthy"`. Nothing in the payload identified
 * the process, so `start.sh` could be satisfied by ANY server holding the port (the
 * live omp-server on :5677) and exit 0 claiming "already running and healthy" while
 * nothing of ours was up. Every response that crosses the port boundary now carries
 * `repo`, and start.sh asserts it with `jq -r .repo == "$REPO_ID"` (W2.3).
 */
import { join } from "node:path";

import { loadSubworkers } from "./auth.ts";

/** Identity of this checkout. Must match `REPO_ID` in start.sh and docs/PLAN.md W3.5. */
export const REPO_ID = "subworker-opencode-rust";

const CONTROL_DIR = new URL(".", import.meta.url).pathname;

/** Repo root — `control/`'s parent. */
export function repoRoot(): string {
  return join(CONTROL_DIR, "..");
}

/** Rust server's data dir, repo-local. The live user db lives under
 *  ~/Library/Application Support/opencode — never point at that. */
export function dbPath(): string {
  return join(repoRoot(), "server", "data");
}

/** Base URL of the child Rust server that `opencode serve` binds (see
 *  server/start-rust-api.sh). Overridable so the control plane can sit in front of a
 *  child on another host/port. */
export function childBase(): string {
  return (process.env.OPENCODE_CHILD_URL?.trim() || "http://127.0.0.1:4096").replace(/\/+$/, "");
}

/** True only when `payload.repo` is exactly REPO_ID — the mirror of start.sh's jq assert. */
export function isOwnRepo(payload: unknown): boolean {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) return false;
  return (payload as { repo?: unknown }).repo === REPO_ID;
}

/** Body of the open `GET /health`. Identity fields first — this is the payload
 *  start.sh, the TopBar, and any operator run `jq` against. */
export function healthPayload(bootAt: number): Record<string, unknown> {
  return {
    status: "ok",
    repo: REPO_ID,
    db_path: dbPath(),
    agents_count: countAgents(),
    uptime_s: Math.floor((Date.now() - bootAt) / 1000),
  };
}

function countAgents(): number {
  try {
    return loadSubworkers().length;
  } catch {
    return 0;
  }
}

/** Probe the child Rust `GET /health`. Never throws: an unreachable child is reported,
 *  not raised, so /server/health can answer with the reason instead of a 500. */
export async function readChildHealth(
  timeoutMs = 2000,
): Promise<{ reachable: boolean; status?: string; version?: string; error?: string }> {
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const res = await fetch(`${childBase()}/health`, { signal: ctl.signal });
    if (!res.ok) return { reachable: false, error: `child /health returned ${res.status}` };
    const body = (await res.json()) as { status?: unknown; version?: unknown };
    return {
      reachable: true,
      status: typeof body.status === "string" ? body.status : undefined,
      version: typeof body.version === "string" ? body.version : undefined,
    };
  } catch (err) {
    return { reachable: false, error: err instanceof Error ? err.message : String(err) };
  } finally {
    clearTimeout(timer);
  }
}

/** `healthy` only when the child answered AND said ok — never a constant. */
export function healthStatusFromChild(child: { reachable: boolean; status?: string }): string {
  if (!child.reachable) return "unreachable";
  return child.status === "ok" ? "healthy" : `degraded:${child.status ?? "unknown"}`;
}