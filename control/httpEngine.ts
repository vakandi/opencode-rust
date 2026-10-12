/**
 * httpEngine.ts — drives the Rust harness over HTTP instead of the old Bun SDK.
 *
 * Wire contract, read off the Rust source rather than assumed:
 *   POST /session?directory=<abs>   body {agent?,title?,permission?}  -> {id}
 *   POST /session/{id}/prompt       body {message,model?,variant?,agent?} -> 200 immediately
 *   GET  /event                    SSE, envelope {id,type,properties}
 *
 * Three facts that dictate the whole design:
 *
 * 1. `session.idle` NEVER reaches a plain SSE client. routes.rs:1763 sends it through
 *    `notify_plugin_event`, which only plugin subscribers see. Awaiting idle means
 *    hanging forever. The only completion signal on the public stream is
 *    `session.updated` carrying `source: "prompt.final"` (routes.rs:1758).
 *
 * 2. `POST /prompt` returns 200 BEFORE the session goes Busy, and the turn is
 *    `tokio::spawn`ed — so it always runs to completion and always emits prompt.final
 *    even if we stop listening. Polling `/session/{id}` is only for recovery, never
 *    the primary signal.
 *
 * 3. `POST /session/{id}/abort` is a no-op (routes.rs:1825-1846). Posting to it and
 *    expecting an effect is a bug; this module never calls it. A timeout abandons the
 *    HTTP request and records "timeout", which is the truth about what we observed.
 */

export interface Frame {
  field: "text" | "reasoning" | "tool";
  text: string;
  tool?: string;
  input?: unknown;
  output?: string;
}

export type Terminal =
  | { event: "done"; status: "completed" }
  | { event: "done"; status: "failed"; error: string }
  | { event: "done"; status: "timeout"; error: string };

export interface RunOptions {
  baseUrl: string;
  directory: string;
  agent?: string | undefined;
  model?: string | undefined;
  prompt: string;
  timeoutMs: number;
  fetchImpl?: typeof fetch;
}

const MAX_TEXT = 8000;

function clamp(text: string): string {
  return text.length > MAX_TEXT ? text.slice(0, MAX_TEXT) + "… (truncated)" : text;
}

/** Parse one SSE `data:` payload, tolerating the `[DONE]` sentinel and blank lines. */
export function parseSseData(raw: string): Record<string, unknown> | null {
  const trimmed = raw.trim();
  if (!trimmed || trimmed === "[DONE]") return null;
  try {
    const value = JSON.parse(trimmed);
    return value && typeof value === "object" ? (value as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/**
 * The Rust server emits `prompt.final` even when the turn failed: on a provider error
 * it appends an assistant text part `"Provider error: …"` and then broadcasts the same
 * completion event (routes.rs:1748 then :1753). So prompt.final proves the turn ENDED,
 * not that it SUCCEEDED. Without this check every failed run is recorded as completed.
 */
export const PROVIDER_ERROR_PREFIX = "Provider error: ";

export function isProviderError(text: string): boolean {
  return text.trimStart().startsWith(PROVIDER_ERROR_PREFIX);
}

/** Strip the prefix so the stored frame carries the bare reason. */
export function providerErrorText(text: string): string {
  return text.trimStart().slice(PROVIDER_ERROR_PREFIX.length).trim();
}

/**
 * True for the prompt the server replays back, not for model output.
 *
 * The replay arrives as an ordinary text part with no marker and no role, and it is the
 * entire content of a dead run. A real answer can be as long as an agent prompt, so
 * length cannot separate them — the "> OMP HOME:" header every subworker prompt starts
 * with can. Measured over the recorded frames: 8 echoes, 0 false positives.
 */
export function isPromptEcho(text: string): boolean {
  return text.startsWith("#") && text.slice(0, 400).includes("> OMP HOME:");
}

/** True only for the one event that proves the turn finished. */
export function isPromptFinal(event: Record<string, unknown>): boolean {
  if (event.type !== "session.updated") return false;
  const props = (event.properties ?? {}) as Record<string, unknown>;
  // The broadcast at routes.rs:1758 puts `source` at the top level; the enveloped
  // path puts it under `properties`. Accept both rather than silently hanging on one.
  return event.source === "prompt.final" || props.source === "prompt.final";
}

/**
 * Map a `message.part.updated` / `message.part.delta` envelope to zero or more frames.
 *
 * `seen` carries the part IDs already announced via `updated`. The Rust differ emits
 * `part.updated` with the part's FULL text on first sight, then a `part.delta` carrying
 * the very same bytes on the next tick. Emitting both renders every chunk twice, so a
 * text or reasoning part is announced once and then streamed by delta only.
 */
export function framesFromEvent(
  event: Record<string, unknown>,
  sessionId: string,
  seen: Set<string> = new Set(),
  callNames: Map<string, string> = new Map(),
): Frame[] {
  const props = (event.properties ?? {}) as Record<string, unknown>;
  if (props.sessionID !== undefined && props.sessionID !== sessionId) return [];

  if (event.type === "message.part.delta") {
    const delta = typeof props.delta === "string" ? props.delta : "";
    if (!delta) return [];
    // The Rust differ emits field "text" for every streamed append. Reasoning arrives as
    // its own part type instead, so a delta is always display text.
    return [{ field: "text", text: delta }];
  }

  if (event.type !== "message.part.updated") return [];
  const part = props.part as Record<string, unknown> | undefined;
  if (!part) return [];
  const partType = part.type;

  if (partType === "text" || partType === "reasoning") {
    const partId = typeof part.id === "string" ? part.id : "";
    if (partId !== "" && seen.has(partId)) return [];
    if (partId !== "") seen.add(partId);
    const text = typeof part.text === "string" ? part.text : "";
    if (!text) return [];
    return [{ field: partType === "text" ? "text" : "reasoning", text: clamp(text) }];
  }
  if (partType === "tool") {
    const state = (part.state ?? {}) as Record<string, unknown>;
    const status = typeof state.status === "string" ? state.status : "";
    const callId = typeof part.callID === "string" ? part.callID : "";

    // A ToolCall part carries the tool name; the matching ToolResult part does not — it
    // only repeats the callID. Correlate by that ID so a result is never rendered as a
    // nameless "▸ tool".
    if (typeof part.tool === "string" && callId !== "") callNames.set(callId, part.tool);
    const tool = (typeof part.tool === "string" ? part.tool : undefined)
      ?? (callId !== "" ? callNames.get(callId) : undefined);
    if (!tool) return [];

    const announceKey = `tool:${callId || (typeof part.id === "string" ? part.id : "")}`;
    // The differ re-sends an unchanged pending ToolCall on later ticks; one "▸ name" per
    // call is enough.
    if (status === "pending") {
      if (seen.has(announceKey)) return [];
      seen.add(announceKey);
      return [{ field: "tool", text: `▸ ${tool}`, tool, input: state.input }];
    }
    if (status === "completed" || status === "error") {
      const resultKey = `${announceKey}:result`;
      if (seen.has(resultKey)) return [];
      seen.add(resultKey);
      const output = typeof state.output === "string" ? state.output : "";
      return [{ field: "tool", text: `▸ ${tool}`, tool, output: clamp(output) }];
    }
  }
  return [];
}

// The Rust child requires the same admin token as the control plane. Without this
// header every call is a 401 and no subworker run ever starts.
function childAuthHeaders(extra: Record<string, string> = {}): Record<string, string> {
  const token = (process.env.ELIA_AUTH_TOKEN ?? "").trim();
  return token ? { ...extra, authorization: `Bearer ${token}` } : extra;
}

async function createSession(opts: RunOptions, doFetch: typeof fetch): Promise<string> {
  const url = `${opts.baseUrl}/session?directory=${encodeURIComponent(opts.directory)}`;
  const res = await doFetch(url, {
    method: "POST",
    headers: childAuthHeaders({ "content-type": "application/json" }),
    body: JSON.stringify({ agent: opts.agent, title: opts.agent }),
  });
  if (!res.ok) {
    throw new Error(`POST /session failed: ${res.status} ${(await res.text()).slice(0, 300)}`);
  }
  const body = (await res.json()) as Record<string, unknown>;
  const id = body.id ?? body.sessionID;
  if (typeof id !== "string" || !id) {
    throw new Error(`POST /session returned no session id: ${JSON.stringify(body).slice(0, 200)}`);
  }
  return id;
}

/**
 * Run one turn and yield frames as they arrive, then exactly one terminal event.
 *
 * The consumer (engine.ts) feeds each frame to the run record and treats the terminal
 * event as the outcome. Yielding the terminal rather than throwing keeps the caller's
 * existing protocol handling unchanged.
 */
export async function* runToFrames(opts: RunOptions): AsyncGenerator<Frame | Terminal> {
  const doFetch = opts.fetchImpl ?? fetch;
  // Frames are handed over as they arrive, not at the end. A TopBar/livestream consumer
  // must paint the run while it happens; buffering until prompt.final would make every
  // run look like it hung and then dumped everything at once.
  const queue: Frame[] = [];
  // prompt.final fires on failure too, so a provider error must be captured to classify
  // the run. Cleared per run; set the first time one is seen.
  let providerError: string | null = null;
  // The session marker and the echoed prompt are not work: only assistant text or a
  // tool call proves the model did something this turn.
  let sawAssistantWork = false;
  const announcedParts = new Set<string>();
  const toolCallNames = new Map<string, string>();
  let wake: (() => void) | null = null;
  const push = (frame: Frame): void => {
    if (frame.field === "tool") sawAssistantWork = true;
    // The prompt echo is replayed to the UI, so it must not count as model output.
    else if (frame.field === "text" && !isPromptEcho(frame.text)) sawAssistantWork = true;
    if (providerError === null && frame.field === "text" && isProviderError(frame.text)) {
      providerError = providerErrorText(frame.text);
    }
    queue.push(frame);
    wake?.();
    wake = null;
  };
  const nextFrame = (): Promise<Frame | null> => {
    if (queue.length > 0) return Promise.resolve(queue.shift() as Frame);
    return new Promise<Frame | null>((resolve) => {
      wake = () => resolve(queue.shift() ?? null);
    });
  };

  let sessionId: string;
  try {
    sessionId = await createSession(opts, doFetch);
  } catch (err) {
    yield { event: "done", status: "failed", error: String(err).slice(0, 300) };
    return;
  }
  yield { field: "text", text: `[session ${sessionId}]` };

  // Subscribe BEFORE prompting. The other order races: a fast turn can emit prompt.final
  // before we are listening, and we would then wait for a timeout on a run that is done.
  const streamAbort = new AbortController();
  let settle: (() => void) | null = null;
  const streamDone = (async (): Promise<"final" | "closed" | "aborted"> => {
    try {
      const res = await doFetch(`${opts.baseUrl}/event`, {
        headers: childAuthHeaders({ accept: "text/event-stream" }),
        signal: streamAbort.signal,
      });
      if (!res.ok || !res.body) return "closed";
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buf = "";
      for (;;) {
        const { done, value } = await reader.read();
        if (value) {
          buf += decoder.decode(value, { stream: true });
          let idx: number;
          while ((idx = buf.indexOf("\n")) >= 0) {
            const line = buf.slice(0, idx).replace(/^data:\s?/, "");
            buf = buf.slice(idx + 1);
            const event = parseSseData(line);
            if (!event) continue;
            if (isPromptFinal(event)) {
              settle?.();
              return "final";
            }
            for (const frame of framesFromEvent(event, sessionId, announcedParts, toolCallNames)) push(frame);
          }
        }
        if (done) break;
      }
      settle?.();
      return "closed";
    } catch {
      settle?.();
      return "aborted";
    }
  })();

  // POST /prompt returns 200 before Busy is set; the turn is already spawned server-side.
  try {
    const res = await doFetch(`${opts.baseUrl}/session/${sessionId}/prompt`, {
      method: "POST",
      headers: childAuthHeaders({ "content-type": "application/json" }),
      body: JSON.stringify({ message: opts.prompt, model: opts.model, agent: opts.agent }),
    });
    if (!res.ok) {
      streamAbort.abort();
      await streamDone;
      yield { event: "done", status: "failed", error: `POST /prompt failed: ${res.status} ${(await res.text()).slice(0, 300)}` };
      return;
    }
  } catch (err) {
    streamAbort.abort();
    await streamDone;
    yield { event: "done", status: "failed", error: String(err).slice(0, 300) };
    return;
  }

  const deadline = Date.now() + opts.timeoutMs;
  let outcome: "final" | "closed" | "aborted" | "timeout" = "timeout";
  const streamSettled = streamDone.then((r) => {
    outcome = r;
    return r;
  });

  for (;;) {
    const remaining = deadline - Date.now();
    const raced = await Promise.race([
      nextFrame().then((frame) => ({ kind: "frame" as const, frame })),
      streamSettled.then((result) => ({ kind: "settled" as const, result })),
      new Promise<{ kind: "timeout" }>((resolve) =>
        setTimeout(() => resolve({ kind: "timeout" }), Math.max(0, remaining)),
      ),
    ]);

    if (raced.kind === "frame") {
      if (raced.frame === null) continue;
      yield raced.frame;
      continue;
    }

    streamAbort.abort();
    for (let f = queue.shift(); f !== undefined; f = queue.shift()) yield f;

    if (raced.kind === "timeout" || raced.result !== "final") {
      if (outcome === "final") {
        // The turn ended, but a provider error means it ended badly.
        if (providerError !== null) {
          yield { event: "done", status: "failed", error: providerError };
          return;
        }
        // A turn that streamed no assistant text and ran no tool produced nothing.
        // Reporting "completed" here is what let a dead run look healthy in the UI.
        if (!sawAssistantWork) {
          yield {
            event: "done",
            status: "failed",
            error:
              "turn ended with no assistant output and no tool call — the model never produced a token",
          };
          return;
        }
        yield { event: "done", status: "completed" };
        return;
      }
      if (raced.kind === "timeout") {
        yield {
          event: "done",
          status: "timeout",
          error: `no prompt.final within ${opts.timeoutMs}ms — request abandoned, turn left running server-side`,
        };
        return;
      }
      yield {
        event: "done",
        status: "failed",
        error: "event stream closed before prompt.final — completion cannot be confirmed",
      };
      return;
    }

    if (providerError !== null) {
      yield { event: "done", status: "failed", error: providerError };
      return;
    }
    yield { event: "done", status: "completed" };
    return;
  }
}
