/** Session routes: /sessions/{name}, /sessions/{name}/list,
 * POST /sessions/{name}/{id}/continue, GET /sessions/{name}/{id}/events (SSE).
 * Shapes match live for LogPopoverView; events route follows the qwen shim shape.
 */
import { getEntry } from "../scheduler.ts";
import { getRun, lastSessionId, readFrames, runningSessionId, upsertRun, nowIso, type Frame } from "../store.ts";
import { broadcast, emitSubworkerCompleted, emitSubworkerStarted } from "../ws.ts";
import { statusPayload } from "../statusView.ts";
import { json, notFound } from "./status.ts";

interface MsgPart {
  type: string;
  text?: string | null;
  tool?: string | null;
  output?: string | null;
}

function bareToolName(frame: Frame): string | null {
  if (frame.tool && frame.tool.length > 0) return frame.tool;
  const m = frame.text.replace(/^[\u25B6\u25B8\u2714\u2718]\s*/, "");
  return m.length > 0 ? m : null;
}

function framesToMessages(frames: Frame[], limit: number): Array<{ info: Record<string, unknown>; parts: MsgPart[] }> {
  const sliced = frames.slice(-limit);
  const texts: string[] = [];
  const reasonings: string[] = [];
  const tools: Frame[] = [];
  for (const f of sliced) {
    if (f.field === "text") texts.push(f.text);
    else if (f.field === "reasoning") reasonings.push(f.text);
    else tools.push(f);
  }
  const messages: Array<{ info: Record<string, unknown>; parts: MsgPart[] }> = [];
  if (texts.length > 0) {
    messages.push({
      info: { role: "assistant", time_created: sliced[sliced.length - 1]?.t ?? Date.now() },
      parts: [{ type: "text", text: texts.join("") }],
    });
  }
  if (reasonings.length > 0) {
    messages.push({
      info: { role: "assistant", time_created: Date.now() },
      parts: [{ type: "reasoning", text: reasonings.join("\n") }],
    });
  }
  for (const f of tools) {
    const name = bareToolName(f);
    if (!name) continue;
    const part: MsgPart = { type: "tool", tool: name };
    if (f.input !== undefined) part.input = f.input;
    if (f.output !== undefined) part.output = f.output;
    messages.push({ info: { role: "assistant", time_created: Date.now() }, parts: [part] });
  }
  return messages;
}

export async function handleGetSession(name: string, url: URL): Promise<Response> {
  const entry = getEntry(name);
  if (!entry) return notFound(name);
  const limit = Math.min(Number.parseInt(url.searchParams.get("limit") ?? "50", 10) || 50, 500);
  const sessionId = url.searchParams.get("session_id") ?? runningSessionId(name) ?? lastSessionId(name) ?? null;
  if (!sessionId) return json({ name, session_id: null, messages: [], total_messages: 0 });
  const messages = framesToMessages(readFrames(sessionId), limit);
  return json({ name, session_id: sessionId, messages, total_messages: messages.length });
}

export async function handleListSessions(name: string): Promise<Response> {
  const entry = getEntry(name);
  if (!entry) return notFound(name);
  const running = runningSessionId(name);
  const last = lastSessionId(name);
  const seen = new Set<string>();
  const sessions: Array<Record<string, unknown>> = [];
  if (running) {
    sessions.push({ session_id: running, title: "▶ Running", agent: entry.agent_id, time_created: null });
    seen.add(running);
  }
  if (last && !seen.has(last)) {
    sessions.push({ session_id: last, title: null, agent: entry.agent_id, time_created: null });
  }
  return json({ name, sessions });
}

export async function handleContinueSession(name: string, sessionId: string, req: Request): Promise<Response> {
  const entry = getEntry(name);
  if (!entry) return notFound(name);
  let body: { message?: string } = {};
  try {
    if (req.headers.get("content-type")?.includes("json")) body = await req.json();
  } catch {
    body = {};
  }
  const message = (body.message ?? "").trim() || "continue the tasks";
  const run = getRun(sessionId);
  if (run) {
    upsertRun({ ...run, name, status: "running", last_frame_at: nowIso() });
  } else {
    const stamp = nowIso();
    upsertRun({ name, session_id: sessionId, status: "running", created_at: stamp, last_frame_at: stamp, started_at: stamp });
  }
  emitSubworkerStarted(name);
  broadcast(statusPayload());
  // re-inject without awaiting — TopBar flips to LIVE via subworker_started
  void import("../engine.ts").then(
    (mod) => mod.continueEngineSession(entry, sessionId, message),
    (err) => console.error(`[sessions] continue failed name=${name} session=${sessionId}: ${String(err)}`),
  );
  return json({ status: "continued", name, session_id: sessionId, message: `Message sent to session ${sessionId}` });
}

export async function handleStopSession(name: string, sessionId: string): Promise<Response> {
  const entry = getEntry(name);
  if (!entry) return notFound(name);
  // Lazy import: engine.ts must never be statically imported outside itself (cycle).
  const { killRun } = await import("../engine.ts");
  killRun(sessionId);
  const run = getRun(sessionId);
  const stamp = nowIso();
  if (run) {
    upsertRun({ ...run, name, status: "cancelled", last_frame_at: stamp });
  } else {
    upsertRun({ name, session_id: sessionId, status: "cancelled", created_at: stamp, last_frame_at: stamp, started_at: stamp });
  }
  emitSubworkerCompleted(name, "cancelled", sessionId);
  broadcast(statusPayload());
  return json({ status: "cancelled", name, session_id: sessionId, message: `Session ${sessionId} cancelled` });
}

/** SSE: replay buffered frames then live-tail. Client disconnect = clean EOF. */
export async function handleSessionEvents(name: string, sessionId: string): Promise<Response> {
  const entry = getEntry(name);
  if (!entry) return notFound(name);
  const encoder = new TextEncoder();
  let closed = false;
  const stream = new ReadableStream({
    start(controller) {
      try {
        for (const f of readFrames(sessionId)) {
          if (closed) break;
          controller.enqueue(encoder.encode(`data: ${JSON.stringify({ name, session_id: sessionId, field: f.field, text: f.text })}\n\n`));
        }
      } catch {
        closed = true;
        try {
          controller.close();
        } catch {
          // already closed
        }
      }
    },
    cancel() {
      closed = true;
    },
  });
  return new Response(stream, {
    headers: {
      "content-type": "text/event-stream",
      "cache-control": "no-cache",
      connection: "keep-alive",
    },
  });
}
