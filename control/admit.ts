/** Run admission — shared by POST /trigger and the scheduler tick.
 * Synchronously (<100ms) creates the run record and fsyncs BEFORE returning.
 * Over concurrency cap → {status:"queued_429"} HTTP 202. Never silent, never 429-empty.
 */
import { activeRunCount, snowflakeSessionId, upsertRun, nowIso } from "./store.ts";
import { maxConcurrentRuns } from "./auth.ts";
import { getEntry } from "./scheduler.ts";
import type { SubworkerEntry } from "./auth.ts";
import { statusPayload } from "./statusView.ts";
import { emitSubworkerStarted, broadcast } from "./ws.ts";
import { notify } from "./notifier.ts";

export interface AdmitOptions {
  prompt?: string | null;
  model?: string | null;
  variant?: string | null;
}

export interface AdmitResult {
  httpStatus: number;
  body: { status: string; name: string; message?: string; session_id?: string };
}

export async function admitRun(name: string, opts: AdmitOptions = {}): Promise<AdmitResult> {
  const entry: SubworkerEntry | undefined = getEntry(name);
  if (!entry) {
    return { httpStatus: 404, body: { status: "error", name, message: `Subworker '${name}' not found` } };
  }
  const sessionId = snowflakeSessionId();
  const stamp = nowIso();
  if (activeRunCount() >= maxConcurrentRuns()) {
    upsertRun({
      name,
      session_id: sessionId,
      status: "queued_429",
      created_at: stamp,
      last_frame_at: stamp,
      started_at: stamp,
      model: opts.model ?? entry.model ?? null,
      variant: opts.variant ?? entry.variant ?? null,
    });
    await notify(name, "queued_429", `admission queued under load (session ${sessionId})`);
    return { httpStatus: 202, body: { status: "queued_429", name, session_id: sessionId } };
  }
  upsertRun({
    name,
    session_id: sessionId,
    status: "running",
    created_at: stamp,
    last_frame_at: stamp,
    started_at: stamp,
    model: opts.model ?? entry.model ?? null,
    variant: opts.variant ?? entry.variant ?? null,
  });
  // queue engine start without awaiting it (engine module loads lazily so the
  // listener boots without the native SDK addon; admission is already persisted)
  void import("./engine.ts").then(
    (mod) => mod.startEngine({ entry, sessionId, promptOverride: opts.prompt, modelOverride: opts.model, variantOverride: opts.variant }),
    (err) => console.error(`[admit] engine load failed session=${sessionId}: ${String(err)}`),
  );
  emitSubworkerStarted(name);
  broadcast(statusPayload());
  return {
    httpStatus: 200,
    body: { status: "triggered", name, message: `Subworker '${name}' triggered successfully`, session_id: sessionId },
  };
}
