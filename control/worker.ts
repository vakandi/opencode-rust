/**
 * ⚠️ SUPERSEDED — NOT SPAWNED. Kept for reference only.
 *
 * This is the old Bun-SDK worker. It imported `./packages/coding-agent`, which is not
 * part of this repo, and nothing spawns it any more: `spawnWorkerAndWait` in engine.ts
 * now calls `runToFrames` from httpEngine.ts, which drives the native Rust harness.
 *
 * Do not wire this back in. Doing so reintroduces the SDK dependency this repo does
 * not have, and the failure is a silent spawn error at run time, not a build error.
 */
import { existsSync, readFileSync } from "node:fs";
import { join, basename } from "node:path";
import { Settings } from "./packages/coding-agent/src/config/settings.ts";
import { getAgentDir } from "./packages/utils/src/dirs.ts";

/** Read a string field off an unknown object with runtime narrowing. */
function strField(obj: object, key: string): string | null {
  if (key in obj) {
    const value: unknown = obj[key as keyof typeof obj];
    if (typeof value === "string") return value;
  }
  return null;
}

/** One TopBar-consumable frame. Field contract fixed; SDK names remapped here. */
export interface EngineFrame {
  field: "text" | "reasoning" | "tool";
  text: string;
  /** Bare tool name for exact-match panel parsers. */
  tool?: string;
  /** Call args object — subagent panels read description/subagent_type/sessionId. */
  input?: unknown;
  /** Truncated result text — subagent panels scan it for session ids. */
  output?: string;
}

const SUBAGENT_TOOLS = new Set(["task", "subtask", "hub", "call_omo_agent", "team_create", "team_task_create"]);

/** toolCallId → synthetic subagent session id (TopBar requires ses_-prefixed ids). */
const subagentSessions = new Map<string, string>();

function newSubSessionId(): string {
  return `ses_${Date.now().toString(36)}${Math.floor(Math.random() * 0xffffff).toString(36).padStart(4, "0")}`;
}

function truncateOutput(value: unknown): string {
  const text = typeof value === "string" ? value : JSON.stringify(value) ?? String(value);
  return text.length > 8000 ? text.slice(0, 8000) + "\n… (truncated)" : text;
}

function taskInput(args: unknown, toolName: string): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  if (args && typeof args === "object" && !Array.isArray(args)) {
    const rec = args as Record<string, unknown>;
    const firstTask = Array.isArray(rec.tasks) && rec.tasks.length > 0 && typeof rec.tasks[0] === "object" && rec.tasks[0] !== null
      ? (rec.tasks[0] as Record<string, unknown>)
      : null;
    const desc = rec.description ?? firstTask?.task ?? rec.prompt ?? (typeof rec.message === "string" ? rec.message : null) ?? toolName;
    if (typeof desc === "string" && desc.length > 0) out.description = desc.slice(0, 200);
    const agent = rec.subagent_type ?? rec.agent ?? firstTask?.agent ?? "task";
    if (typeof agent === "string" && agent.length > 0) out.subagent_type = agent;
  } else {
    out.description = toolName;
    out.subagent_type = "task";
  }
  return out;
}

/** Cap arbitrary args to a JSON-safe value (TopBar parses dict or JSON string). */
function capJson(value: unknown, maxChars: number): unknown {
  if (value && typeof value === "object") {
    try {
      const text = JSON.stringify(value);
      if (text.length <= maxChars) return value;
      return { _truncated: text.slice(0, maxChars) };
    } catch {
      return { _unstringifiable: true };
    }
  }
  return value;
}
function nonEmpty(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

/** Map one SDK session event to zero or more TopBar frames. TOTAL: every path returns an array. */
function eventToFrames(event: unknown): EngineFrame[] {
  if (!event || typeof event !== "object") return [];
  const type = strField(event, "type") ?? "";
  // Streaming deltas: the incremental payload lives in assistantMessageEvent
  // (message.content is cumulative — emitting it here would duplicate every
  // previous delta into the frame log). Map each sub-event kind to its field.
  if (type === "message_update") {
    if ("assistantMessageEvent" in event && event.assistantMessageEvent && typeof event.assistantMessageEvent === "object") {
      const sub = event.assistantMessageEvent;
      const subtype = strField(sub, "type") ?? "";
      if (subtype === "text_delta") {
        const d = "delta" in sub ? nonEmpty(sub.delta) : null;
        return d ? [{ field: "text", text: d }] : [];
      }
      if (subtype === "text_end") {
        const d = "content" in sub ? nonEmpty(sub.content) : null;
        return d ? [{ field: "text", text: d }] : [];
      }
      if (subtype === "thinking_delta") {
        const d = "delta" in sub ? nonEmpty(sub.delta) : null;
        return d ? [{ field: "reasoning", text: d }] : [];
      }
      if (subtype === "thinking_end") {
        const d = "content" in sub ? nonEmpty(sub.content) : null;
        return d ? [{ field: "reasoning", text: d }] : [];
      }
      if (subtype === "toolcall_end") {
        if ("toolCall" in sub && sub.toolCall && typeof sub.toolCall === "object") {
          const name = strField(sub.toolCall, "name");
          if (name) return [{ field: "tool", text: `▸ ${name}` }];
        }
        return [];
      }
      // start | text_start | thinking_start | toolcall_start | toolcall_delta |
      // image_end | done | error carry no incremental display text.
      return [];
    }
    return [];
  }
  if (type === "message" || type === "message_start" || type === "message_delta" || type === "message_end") {
    const holder: unknown =
      "message" in event && event.message && typeof event.message === "object"
        ? event.message
        : "delta" in event && event.delta && typeof event.delta === "object"
          ? event.delta
          : event;
    if (holder && typeof holder === "object" && "content" in holder) {
      const content: unknown = holder.content;
      if (typeof content === "string" && content.length > 0) {
        // skip system prelude chatter on start; stream real deltas
        if (type === "message_start" && holder && typeof holder === "object" && "customType" in holder) return [];
        return [{ field: "text", text: content }];
      }
      if (Array.isArray(content)) {
        const texts: string[] = [];
        const reasoning: string[] = [];
        const tools: string[] = [];
        for (const block of content) {
          if (!block || typeof block !== "object" || !("type" in block)) continue;
          if (block.type === "text" && "text" in block && typeof block.text === "string" && block.text.length > 0) {
            texts.push(block.text);
          } else if (block.type === "thinking" && "thinking" in block && typeof block.thinking === "string" && block.thinking.length > 0) {
            reasoning.push(block.thinking);
          } else if (block.type === "toolCall" && "name" in block && typeof block.name === "string" && block.name.length > 0) {
            tools.push(`▸ ${block.name}`);
          }
        }
        const out: EngineFrame[] = [];
        if (texts.length > 0) out.push({ field: "text", text: texts.join("") });
        if (reasoning.length > 0) out.push({ field: "reasoning", text: reasoning.join("") });
        for (const t of tools) out.push({ field: "tool", text: t });
        if (out.length > 0) return out;
      }
    }
    return [];
  }
  // Tool lifecycle: name + status line; subagent tools also carry input/output
  // for the TopBar subagents panel (never full unrelated payloads).
  if (type === "tool_execution_start") {
    const name = strField(event, "toolName");
    if (!name) return [];
    const frame: EngineFrame = { field: "tool", text: `▶ ${name}`, tool: name };
    if ("args" in event && event.args !== undefined) frame.input = capJson(event.args, 2048);
    if (SUBAGENT_TOOLS.has(name.toLowerCase()) && "toolCallId" in event && typeof event.toolCallId === "string") {
      const subId = newSubSessionId();
      subagentSessions.set(event.toolCallId, subId);
      const input = taskInput("args" in event ? event.args : undefined, name);
      input.sessionId = subId;
      frame.input = input;
    }
    return [frame];
  }
  if (type === "tool_execution_end") {
    const name = strField(event, "toolName");
    if (!name) return [];
    const failed = "isError" in event && event.isError === true;
    const frame: EngineFrame = { field: "tool", text: `${failed ? "✘" : "✔"} ${name}`, tool: name };
    if ("result" in event && event.result !== undefined) frame.output = truncateOutput(event.result);
    if (SUBAGENT_TOOLS.has(name.toLowerCase()) && "toolCallId" in event && typeof event.toolCallId === "string") {
      const subId = subagentSessions.get(event.toolCallId);
      if (subId) {
        const prev = frame.input;
        frame.input = { sessionId: subId, ...(prev && typeof prev === "object" ? prev as Record<string, unknown> : {}) };
        subagentSessions.delete(event.toolCallId);
      }
    }
    return [frame];
  }
  // notices, compaction/retry/todo/advisor/IRC session events: no frame text.
  // Completion is handled by the subscriber's agent_end branch, not here.
  return [];
}

const [sessionId, workspace, promptFile, modelRaw, agentNameRaw] = Bun.argv.slice(2);

/** Flush stdout (piped protocol lines) before settling the exit code. */
async function flushStdout(): Promise<void> {
  try {
    await Bun.write(Bun.stdout, "");
  } catch {
    // best-effort
  }
  await Bun.sleep(200);
}

/** Load the OMP agent persona (<agentDir>/agents/<name>.md, frontmatter stripped)
 * as appended system prompt. Missing/empty file = skip silently (run prompt
 * still carries the full task). Capped so a bloated persona can't eat context. */
function loadAgentPersona(agentDir: string, name: string): string | undefined {
  const safe = basename(name).replace(/[^a-zA-Z0-9_-]/g, "");
  if (!safe) return undefined;
  let raw: string;
  try {
    raw = readFileSync(join(agentDir, "agents", `${safe}.md`), "utf8");
  } catch {
    return undefined;
  }
  const body = raw.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, "").trim();
  if (!body) return undefined;
  if (body.length > 12_000) {
    console.error(`[worker] persona ${safe}.md exceeds 12k chars (${body.length}); truncated`);
    return body.slice(0, 12_000);
  }
  return body;
}

async function main(): Promise<void> {
  if (!sessionId || !workspace || !promptFile) {
    console.log(JSON.stringify({ event: "done", status: "failed", error: "worker usage: bun omp-server/worker.ts <sessionId> <workspace> <promptFile> [model]" }));
    await flushStdout();
    process.exit(1);
  }

  let prompt: string;
  try {
    prompt = readFileSync(promptFile, "utf8");
  } catch (err) {
    console.log(JSON.stringify({ event: "done", status: "failed", error: `prompt file unreadable: ${String(err).slice(0, 300)}` }));
    await flushStdout();
    process.exit(1);
  }

  let model: string | undefined;
  const raw = (modelRaw ?? "").trim();
  if (raw) {
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed === "string") model = parsed || undefined;
      else if (parsed && typeof parsed === "object" && "model" in parsed && typeof (parsed as { model: unknown }).model === "string") {
        model = ((parsed as { model: string }).model as string) || undefined;
      } else model = raw;
    } catch {
      model = raw;
    }
  }

  // SDK via dual-layout loader (host `omp-server/packages` vs image `packages`).
  const { loadSdk } = await import("./engine.ts");
  const { createAgentSession, EventBus } = await loadSdk();

  // Vendored proxy-local ONLY (explicit-only discovery): the container gets
  // OUR copy (native rotation, ELIA_PROXY_LOG), never the host-mounted one.
  // Candidates cover image (/srv/omp-server) and host checkout layouts.
  const extCandidates = ["/srv/omp-server/extensions", join(process.cwd(), "omp-server", "extensions")];
  const extDir = extCandidates.find((d) => existsSync(join(d, "proxy-local.ts")));
  // Trimmed runtime: browser tabs + desktop control off (hunters use
  // web_search/fetch); MCP/memory/extensions untouched (prompts may use them).
  const trimmed = await Settings.init({
    cwd: workspace,
    overrides: { "browser.enabled": false, "computer.enabled": false },
  });
  const eventBus = new EventBus();
  const resolvedAgentDir = process.env.OMP_AGENT_DIR?.trim() || getAgentDir();
  const agentName = (agentNameRaw ?? "").trim();
  const persona = agentName ? loadAgentPersona(resolvedAgentDir, agentName) : undefined;
  console.error(`[worker] session=${sessionId} agent=${agentName || "n/a"} persona=${persona ? `${persona.length} chars` : "none"}`);
  const { session } = await createAgentSession({
    cwd: workspace,
    agentDir: process.env.OMP_AGENT_DIR?.trim() || undefined,
    eventBus,
    settings: trimmed,
    ...(persona ? { appendSystemPrompt: persona } : {}),
    // Model string goes here, NOT in session.prompt opts (AgentPromptOptions
    // has no model field — it is silently ignored there). modelPattern is the
    // CLI-blessed deferred path: extension-provided models (e.g. zen-free/*
    // from models.yml) register after extensions load, so the pattern resolves
    // post-extension instead of falling back to the default provider model.
    ...(model ? { modelPattern: model } : {}),
    ...(extDir ? { disableExtensionDiscovery: true, additionalExtensionPaths: [extDir] } : {}),
  });
  if (!extDir) console.error("[worker] WARNING: no vendored extensions dir found; discovery left default");

  let doneSent = false;
  const sendDone = async (status: "completed" | "failed", error?: string): Promise<void> => {
    if (doneSent) return;
    doneSent = true;
    console.log(JSON.stringify(error ? { event: "done", status, error } : { event: "done", status }));
    try {
      await session.dispose();
    } catch {
      // dispose best-effort; run record already final
    }
    await flushStdout();
    process.exit(status === "completed" ? 0 : 1);
  };


  session.subscribe((event) => {
    try {
      const frames = eventToFrames(event) ?? [];
      for (const f of frames) {
        console.log(JSON.stringify({ field: f.field, text: f.text, tool: f.tool, input: f.input, output: f.output }));
      }
      const etype = event && typeof event === "object" && "type" in event ? (event as { type: unknown }).type : undefined;
      if (etype === "agent_end") void sendDone("completed");
    } catch (err) {
      console.error(`[worker] frame handling failed session=${sessionId}: ${String(err)}`);
    }
  });

  try {
    const ok = await session.prompt(prompt, model ? { model } : undefined);
    if (!ok) throw new Error("prompt rejected by session");
    // prompt() resolves when the turn is admitted; wait for idle = turn done.
    await session.waitForIdle();
    await sendDone("completed");
  } catch (err) {
    console.error(`[worker] fatal session=${sessionId}: ${String(err)}`);
    await sendDone("failed", String(err instanceof Error ? err.message : err).slice(0, 500));
  }
}

await main();
