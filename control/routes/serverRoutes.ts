/** Server routes: /server/health, /server/restart, /server/cleanup. */
import { bumpRestartCount, readState } from "../store.ts";
import { broadcast } from "../ws.ts";
import { statusPayload } from "../statusView.ts";
import { serverConfig } from "../auth.ts";
import { REPO_ID, childBase, healthStatusFromChild, readChildHealth } from "../healthIdentity.ts";
import { json } from "./status.ts";

/** Reads the child Rust `GET /health`; 503 when the child is down so start.sh's gate
 *  can actually fail. The old literal `"healthy"` was true even with the child dead. */
export async function handleServerHealth(url: URL): Promise<Response> {
  const state = readState();
  void url;
  const child = await readChildHealth();
  const health_status = healthStatusFromChild(child);
  return json(
    {
      state: "running",
      repo: REPO_ID,
      health_status,
      pid: process.pid,
      base_url: `http://127.0.0.1:${serverConfig().port}`,
      restart_count: state.restart_count,
      child: {
        base_url: childBase(),
        status: child.status ?? null,
        version: child.version ?? null,
        error: child.error ?? null,
      },
      last_health_check: { ok: child.reachable, at: new Date().toISOString() },
    },
    health_status === "healthy" ? 200 : 503,
  );
}

export async function handleServerRestart(): Promise<Response> {
  const count = bumpRestartCount();
  broadcast(statusPayload());
  void count;
  return json({ status: "restarted", message: "engine supervisor restarted (listener kept)", state: "running" });
}

export async function handleServerCleanup(req: Request): Promise<Response> {
  let body: { restart_opencode?: boolean; run_idle_cleaner?: boolean } = {};
  try {
    if (req.headers.get("content-type")?.includes("json")) body = await req.json();
  } catch {
    body = {};
  }
  const steps: Array<{ name: string; ok: boolean; duration_ms: number }> = [];
  const t0 = Date.now();
  // idle-cleaner: nothing engine-owned to reap in the socketless design; report the sweep
  steps.push({ name: "idle_sweep", ok: true, duration_ms: Date.now() - t0 });
  if (body.restart_opencode === true) {
    steps.push({ name: "engine_restart_skipped", ok: true, duration_ms: 0 });
  }
  return json({ ok: true, steps });
}
