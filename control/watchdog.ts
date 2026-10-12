/** Stall watchdog — no RSS-cap gate anywhere (container mem_limit is the only guard).
 * Per run: last_frame_at + started_at. STALL_TIMEOUT_S default 300, MAX_RUN_S default 3600.
 * On expiry: mark failed, error banner via /ws, persist, Discord alert. Never eternal "running".
 */
import { readRuns, upsertRun, nowIso } from "./store.ts";
import { emitRunBanner, emitSubworkerCompleted } from "./ws.ts";
import { notify } from "./notifier.ts";
import { releaseSessionProxy } from "./store.ts";

let runKiller: ((sessionId: string) => boolean) | null = null;

/** Register the engine's child killer (called on stall/max expiry before marking failed). */
export function setRunKiller(fn: (sessionId: string) => boolean): void {
  runKiller = fn;
}

export function stallTimeoutS(): number {
  const raw = Number.parseInt(process.env.STALL_TIMEOUT_S ?? "", 10);
  return Number.isFinite(raw) && raw > 0 ? raw : 300;
}

export function maxRunS(): number {
  const raw = Number.parseInt(process.env.MAX_RUN_S ?? "", 10);
  return Number.isFinite(raw) && raw > 0 ? raw : 3600;
}

export async function checkOnce(nowMs = Date.now()): Promise<string[]> {
  const expired: string[] = [];
  for (const run of readRuns()) {
    if (run.status !== "running" && run.status !== "continued") continue;
    const lastFrame = Date.parse(run.last_frame_at || run.started_at || run.created_at);
    const started = Date.parse(run.started_at || run.created_at);
    if (Number.isNaN(lastFrame) || Number.isNaN(started)) continue;
    const idleS = (nowMs - lastFrame) / 1000;
    const ageS = (nowMs - started) / 1000;
    let reason: string | null = null;
    if (idleS >= stallTimeoutS()) reason = `stall: no frames in ${Math.round(idleS)}s`;
    else if (ageS >= maxRunS()) reason = `max runtime exceeded: ${Math.round(ageS)}s`;
    if (!reason) continue;
    if (runKiller) {
      let killed = false;
      try {
        killed = runKiller(run.session_id);
      } catch (err) {
        console.error(`[watchdog] killer failed session=${run.session_id}: ${String(err)}`);
      }
      console.log(killed ? `[watchdog] killed session=${run.session_id}` : `[watchdog] no child for session=${run.session_id}`);
    }
    upsertRun({ ...run, status: "failed", error: reason, last_frame_at: nowIso() });
    emitRunBanner(run.name, { kind: "error", error: reason });
    emitSubworkerCompleted(run.name, "failed", run.session_id);
    releaseSessionProxy(run.session_id);
    await notify(run.name, "stall", `${reason} (session ${run.session_id})`);
    expired.push(run.session_id);
  }
  return expired;
}

export function startWatchdog(intervalMs = 30_000): void {
  const timer = setInterval(() => {
    checkOnce().catch((err) => console.error(`[watchdog] check failed: ${String(err)}`));
  }, intervalMs);
  timer.unref?.();
}
