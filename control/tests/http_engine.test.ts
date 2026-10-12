/**
 * http_engine.test.ts — proves the three invariants that a screenshot cannot show.
 *
 * Uses a fake HTTP server on a real localhost port so the SSE framing, the abort
 * semantics and the timeout path are exercised for real. It never touches the live
 * Rust server and never POSTs to /abort.
 */
import { test, expect } from "bun:test";
import { runToFrames, isPromptFinal, parseSseData, framesFromEvent, type Frame, type Terminal } from "../httpEngine.ts";

interface FakeOptions {
  /** SSE payloads pushed after the prompt POST lands. */
  events: Array<Record<string, unknown>>;
  /** Never send a terminal event — used to prove the timeout path. */
  omitFinal?: boolean;
}

function startFakeServer(opts: FakeOptions): { baseUrl: string; close: () => void; abortHits: number; promptHits: number } {
  const state = { abortHits: 0, promptHits: 0 };
  const server = Bun.serve({
    port: 0,
    async fetch(req) {
      const url = new URL(req.url);
      if (url.pathname === "/session" && req.method === "POST") {
        return Response.json({ id: "ses_test123" });
      }
      if (url.pathname === "/event") {
        let closed = false;
        req.signal.addEventListener("abort", () => { closed = true; });
        const stream = new ReadableStream<Uint8Array>({
          async start(controller) {
            const enc = new TextEncoder();
            const send = (o: Record<string, unknown>) =>
              controller.enqueue(enc.encode(`data: ${JSON.stringify(o)}\n\n`));
            send({ type: "server.connected" });
            // Wait for the prompt POST before streaming anything.
            while (state.promptHits === 0 && !closed) await Bun.sleep(5);
            for (const e of opts.events) {
              if (closed) return;
              send(e);
              await Bun.sleep(5);
            }
            if (!opts.omitFinal && !closed) {
              send({ type: "session.updated", sessionID: "ses_test123", source: "prompt.final" });
            }
            while (!closed) await Bun.sleep(20);
          },
        });
        return new Response(stream, {
          headers: { "content-type": "text/event-stream", "cache-control": "no-cache" },
        });
      }
      if (url.pathname === "/session/ses_test123/prompt") {
        state.promptHits++;
        return Response.json({ ok: true });
      }
      if (url.pathname.endsWith("/abort")) {
        state.abortHits++;
        return Response.json({ ok: true });
      }
      return new Response("not found", { status: 404 });
    },
  });
  return {
    baseUrl: `http://127.0.0.1:${server.port}`,
    close: () => server.stop(true),
    get abortHits() { return state.abortHits; },
    get promptHits() { return state.promptHits; },
  };
}

async function drain(gen: AsyncGenerator<Frame | Terminal>): Promise<{ frames: Frame[]; terminal: Terminal | null }> {
  const frames: Frame[] = [];
  let terminal: Terminal | null = null;
  for await (const item of gen) {
    if ("event" in item) terminal = item as Terminal;
    else frames.push(item as Frame);
  }
  return { frames, terminal };
}

test("resolves on prompt.final and yields the tool + text frames before it", async () => {
  const fake = startFakeServer({
    events: [
      { id: "e1", type: "message.part.updated", properties: { sessionID: "ses_test123", part: { type: "tool", tool: "bash", state: { status: "pending", input: { command: "date" } } } } },
      { id: "e2", type: "message.part.delta", properties: { sessionID: "ses_test123", field: "text", delta: "hello from rust" } },
      { id: "e3", type: "message.part.updated", properties: { sessionID: "ses_test123", part: { type: "tool", tool: "bash", state: { status: "completed", output: "Tue Oct  6" } } } },
    ],
  });
  try {
    const { frames, terminal } = await drain(
      runToFrames({
        baseUrl: fake.baseUrl,
        directory: "/tmp/ws",
        agent: "test-delegator",
        prompt: "go",
        timeoutMs: 10_000,
      }),
    );
    expect(terminal).toEqual({ event: "done", status: "completed" });
    const tools = frames.filter((f) => f.field === "tool");
    expect(tools.length).toBe(2);
    expect(tools[0]?.tool).toBe("bash");
    expect(tools[0]?.input).toEqual({ command: "date" });
    expect(tools[1]?.output).toBe("Tue Oct  6");
    expect(frames.some((f) => f.field === "text" && f.text === "hello from rust")).toBe(true);
    expect(fake.promptHits).toBe(1);
  } finally {
    fake.close();
  }
});

test("an idle-only stream HANGS and the timeout fires — we never await idle", async () => {
  const fake = startFakeServer({
    events: [{ id: "e1", type: "session.updated", sessionID: "ses_test123", source: "session.idle" }],
    omitFinal: true,
  });
  try {
    const started = Date.now();
    const { terminal } = await drain(
      runToFrames({
        baseUrl: fake.baseUrl,
        directory: "/tmp/ws",
        agent: "test-delegator",
        prompt: "go",
        timeoutMs: 600,
      }),
    );
    const elapsed = Date.now() - started;
    expect(elapsed).toBeGreaterThanOrEqual(500);
    expect(elapsed).toBeLessThan(5000);
    expect(terminal).not.toBeNull();
    expect((terminal as Terminal).status).toBe("timeout");
  } finally {
    fake.close();
  }
});

test("timeout records status timeout, never success", async () => {
  const fake = startFakeServer({ events: [], omitFinal: true });
  try {
    const { terminal } = await drain(
      runToFrames({ baseUrl: fake.baseUrl, directory: "/tmp/ws", prompt: "go", timeoutMs: 400 }),
    );
    expect(terminal).toEqual({
      event: "done",
      status: "timeout",
      error: expect.stringContaining("no prompt.final"),
    });
    expect(JSON.stringify(terminal)).not.toContain("completed");
  } finally {
    fake.close();
  }
});

test("never POSTs to /abort", async () => {
  const fake = startFakeServer({ events: [], omitFinal: true });
  try {
    await drain(runToFrames({ baseUrl: fake.baseUrl, directory: "/tmp/ws", prompt: "go", timeoutMs: 300 }));
    expect(fake.abortHits).toBe(0);
  } finally {
    fake.close();
  }
});

test("frames stream LIVE, not buffered until prompt.final", async () => {
  const fake = startFakeServer({
    events: [
      { id: "e1", type: "message.part.delta", properties: { sessionID: "ses_test123", delta: "first" } },
      { id: "e2", type: "message.part.delta", properties: { sessionID: "ses_test123", delta: "second" } },
    ],
  });
  try {
    const gen = runToFrames({
      baseUrl: fake.baseUrl,
      directory: "/tmp/ws",
      prompt: "go",
      timeoutMs: 10_000,
    });
    // Consume one frame at a time and record WHEN it arrived relative to the others.
    const arrivals: Array<{ text: string; at: number }> = [];
    const start = Date.now();
    for await (const item of gen) {
      if ("event" in item) break;
      arrivals.push({ text: (item as Frame).text, at: Date.now() - start });
    }
    const texts = arrivals.map((a) => a.text);
    expect(texts).toContain("first");
    expect(texts).toContain("second");
    // "second" must be yielded while the stream is still open. If the engine buffered,
    // both would appear in one burst at the very end after prompt.final.
    expect(arrivals.length).toBeGreaterThanOrEqual(2);
    const firstAt = arrivals.find((a) => a.text === "first")!.at;
    const secondAt = arrivals.find((a) => a.text === "second")!.at;
    expect(secondAt).toBeGreaterThan(firstAt);
    expect(secondAt).toBeLessThan(5000);
  } finally {
    fake.close();
  }
});

test("stream closing early is a failure, not a silent success", async () => {
  const server = Bun.serve({
    port: 0,
    async fetch(req) {
      const url = new URL(req.url);
      if (url.pathname === "/session") return Response.json({ id: "ses_test123" });
      if (url.pathname === "/event") return new Response("data: {\"type\":\"server.connected\"}\n\n", {
        headers: { "content-type": "text/event-stream" },
      });
      if (url.pathname.endsWith("/prompt")) return Response.json({ ok: true });
      return new Response("nope", { status: 404 });
    },
  });
  try {
    const { terminal } = await drain(
      runToFrames({ baseUrl: `http://127.0.0.1:${server.port}`, directory: "/tmp/ws", prompt: "go", timeoutMs: 5000 }),
    );
    expect((terminal as Terminal).status).toBe("failed");
    expect((terminal as Terminal).error).toContain("before prompt.final");
  } finally {
    server.stop(true);
  }
});

test("isPromptFinal accepts both the broadcast and the enveloped shape", () => {
  expect(isPromptFinal({ type: "session.updated", source: "prompt.final" })).toBe(true);
  expect(isPromptFinal({ type: "session.updated", properties: { source: "prompt.final" } })).toBe(true);
  expect(isPromptFinal({ type: "session.updated", source: "session.idle" })).toBe(false);
  expect(isPromptFinal({ type: "message.part.delta", properties: { source: "prompt.final" } })).toBe(false);
});

test("frames from another session are dropped", () => {
  expect(
    framesFromEvent(
      { type: "message.part.delta", properties: { sessionID: "ses_other", delta: "leak" } },
      "ses_mine",
    ),
  ).toEqual([]);
  expect(parseSseData("[DONE]")).toBeNull();
});
test("a provider error makes prompt.final resolve as FAILED, not completed", async () => {
  const fake = startFakeServer({
    events: [
      {
        id: "e1",
        type: "message.part.updated",
        properties: {
          sessionID: "ses_test123",
          part: { type: "text", text: "Provider error: API error: 403 Forbidden: FreeTierError" },
        },
      },
    ],
  });
  try {
    const { terminal } = await drain(
      runToFrames({ baseUrl: fake.baseUrl, directory: "/tmp/ws", prompt: "go", timeoutMs: 10_000 }),
    );
    // The stream DOES send prompt.final — the run must still not be recorded as success.
    expect((terminal as Terminal).status).toBe("failed");
    expect((terminal as Terminal).error).toContain("403 Forbidden");
    expect((terminal as Terminal).error).not.toContain("Provider error:");
  } finally {
    fake.close();
  }
});

test("a clean turn is still completed", async () => {
  const fake = startFakeServer({
    events: [
      { id: "e1", type: "message.part.delta", properties: { sessionID: "ses_test123", delta: "all good" } },
    ],
  });
  try {
    const { terminal } = await drain(
      runToFrames({ baseUrl: fake.baseUrl, directory: "/tmp/ws", prompt: "go", timeoutMs: 10_000 }),
    );
    expect(terminal).toEqual({ event: "done", status: "completed" });
  } finally {
    fake.close();
  }
});

test("a text part announced by updated is not re-emitted by the first delta", () => {
  const seen = new Set<string>();
  const updated = {
    type: "message.part.updated",
    properties: {
      sessionID: "ses_test123",
      part: { id: "prt_1", type: "text", text: "Hello" },
    },
  };
  const delta = {
    type: "message.part.delta",
    properties: { sessionID: "ses_test123", field: "text", partID: "prt_1", delta: " world" },
  };
  expect(framesFromEvent(updated, "ses_test123", seen)).toEqual([{ field: "text", text: "Hello" }]);
  // The delta carries NEW bytes only — it must still stream.
  expect(framesFromEvent(delta, "ses_test123", seen)).toEqual([{ field: "text", text: " world" }]);
});

test("reasoning parts de-duplicate the same way", () => {
  const seen = new Set<string>();
  const e = { type: "message.part.updated", properties: { sessionID: "ses_test123", part: { id: "prt_r", type: "reasoning", text: "think" } } };
  expect(framesFromEvent(e, "ses_test123", seen)).toEqual([{ field: "reasoning", text: "think" }]);
  expect(framesFromEvent(e, "ses_test123", seen)).toEqual([]);
});

test("a tool result is named from its call, never a nameless 'tool'", () => {
  const seen = new Set<string>();
  const names = new Map<string, string>();
  const call = {
    type: "message.part.updated",
    properties: {
      sessionID: "ses_test123",
      part: { id: "prt_a", type: "tool", callID: "call_1", tool: "bash", state: { status: "pending", input: { command: "pwd" } } },
    },
  };
  // The Rust ToolResult part has NO `tool` field — only the callID.
  const result = {
    type: "message.part.updated",
    properties: {
      sessionID: "ses_test123",
      part: { id: "prt_b", type: "tool", callID: "call_1", state: { status: "completed", output: "/tmp/ws" } },
    },
  };
  expect(framesFromEvent(call, "ses_test123", seen, names)[0]?.tool).toBe("bash");
  const out = framesFromEvent(result, "ses_test123", seen, names);
  expect(out[0]?.tool).toBe("bash");
  expect(out[0]?.text).not.toContain("▸ tool");
  expect(out[0]?.output).toBe("/tmp/ws");
});

test("a repeated pending tool call announces only once", () => {
  const seen = new Set<string>();
  const names = new Map<string, string>();
  const call = {
    type: "message.part.updated",
    properties: { sessionID: "ses_test123", part: { id: "prt_a", type: "tool", callID: "call_9", tool: "bash", state: { status: "pending", input: {} } } },
  };
  expect(framesFromEvent(call, "ses_test123", seen, names)).toHaveLength(1);
  expect(framesFromEvent(call, "ses_test123", seen, names)).toHaveLength(0);
});

test("a tool call yields one announce frame then one result frame, not two announces", () => {
  const seen = new Set<string>();
  const names = new Map<string, string>();
  const pending = { type: "message.part.updated", properties: { sessionID: "s", part: { id: "p1", type: "tool", callID: "c1", tool: "bash", state: { status: "pending", input: { command: "pwd" } } } } };
  const result  = { type: "message.part.updated", properties: { sessionID: "s", part: { id: "p2", type: "tool", callID: "c1", state: { status: "completed", output: "/ws" } } } };
  const a = framesFromEvent(pending, "s", seen, names);
  const b = framesFromEvent(result, "s", seen, names);
  const c = framesFromEvent(pending, "s", seen, names);
  expect(a).toHaveLength(1);
  expect(a[0]?.input).toEqual({ command: "pwd" });
  expect(b).toHaveLength(1);
  expect(b[0]?.output).toBe("/ws");
  // The differ re-sends the pending call on later ticks — must not re-announce.
  expect(c).toHaveLength(0);
});
