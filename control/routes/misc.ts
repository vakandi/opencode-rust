/** Tunnel + models + main-agent routes. */
import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { startSetup, tunnelCheck, tunnelRemove, tunnelStatus, tunnelStop } from "../tunnel.ts";
import { getModels, refreshModelsIfDue } from "../models.ts";
import { getEntry } from "../scheduler.ts";
import { appendFrame, nowIso, snowflakeSessionId, upsertRun } from "../store.ts";
import { broadcast, emitRunBanner, emitRunLog, emitSubworkerStarted } from "../ws.ts";
import { statusPayload } from "../statusView.ts";
import { json } from "./status.ts";

const DATA_DIR = new URL("../data/", import.meta.url).pathname;
const MAIN_AGENT_PATH = join(DATA_DIR, "main-agent.json");

export async function handleTunnelStatus(): Promise<Response> {
  return json(await tunnelStatus());
}

export async function handleTunnelCheck(req: Request): Promise<Response> {
  let body: { domain?: string; api_token?: string; token?: string; global_key?: string; email?: string } = {};
  try {
    body = await req.json();
  } catch {
    return json({ token_ok: false, message: "invalid JSON body" }, 422);
  }
  const apiToken = body.api_token ?? body.token ?? "";
  return json(await tunnelCheck(body.domain ?? "", apiToken, body.global_key ?? "", body.email ?? ""));
}

export async function handleTunnelSetup(req: Request): Promise<Response> {
  let body: { domain?: string; api_token?: string; token?: string; global_key?: string; email?: string } = {};
  try {
    body = await req.json();
  } catch {
    return json({ status: "error", message: "invalid JSON body" }, 422);
  }
  const domain = (body.domain ?? "").trim();
  if (!domain) return json({ status: "error", message: "domain required" }, 422);
  const apiToken = (body.api_token ?? body.token ?? "").trim();
  const globalKey = (body.global_key ?? "").trim();
  const email = (body.email ?? "").trim();
  if (!apiToken && !globalKey) return json({ status: "error", message: "Missing API token or Global API Key." });
  try {
    return json(startSetup(domain, apiToken, { globalKey, email }));
  } catch (exc) {
    return json({ status: "error", message: exc instanceof Error ? exc.message : String(exc) }, 409);
  }
}

export async function handleTunnelStop(): Promise<Response> {
  return json(await tunnelStop());
}

export async function handleTunnelRemove(): Promise<Response> {
  return json(await tunnelRemove());
}

export async function handleModels(): Promise<Response> {
  // Dynamic Zen roster (models.ts). Fire-and-forget refresh when due so cold
  // containers converge without blocking the route. Never throws.
  refreshModelsIfDue().catch(() => {});
  const models = getModels();
  return json({ models, total: models.length });
}

function readMainAgent(): string {
  try {
    if (existsSync(MAIN_AGENT_PATH)) {
      const parsed = JSON.parse(readFileSync(MAIN_AGENT_PATH, "utf8"));
      if (parsed && typeof parsed.name === "string" && parsed.name) return parsed.name;
    }
  } catch {
    // fall through to default
  }
  return "elia";
}

export async function handleGetMainAgent(): Promise<Response> {
  return json({ name: readMainAgent() });
}

export async function handleTestFrames(name: string): Promise<Response> {
  if (!getEntry(name)) return json({ detail: `unknown subworker ${name}` }, 404);
  const sessionId = snowflakeSessionId();
  const stamp = nowIso();
  upsertRun({ name, session_id: sessionId, status: "running", created_at: stamp, last_frame_at: stamp, started_at: stamp });
  emitSubworkerStarted(name);
  const frames: Array<["text" | "reasoning" | "tool", string]> = [
    ["text", "synthetic text frame: the engine bridge is streaming correctly."],
    ["reasoning", "synthetic reasoning frame: considering the plan, weighing alternatives, proceeding stepwise."],
    ["tool", "read completed"],
  ];
  for (const [field, text] of frames) {
    appendFrame(sessionId, field, text);
    emitRunLog(name, field, text);
  }
  emitRunBanner(name, { kind: "info", delaySeconds: 0 });
  broadcast(statusPayload());
  return json({ status: "emitted", name, session_id: sessionId });
}

export async function handleSetMainAgent(req: Request): Promise<Response> {
  let body: { name?: string } = {};
  try {
    body = await req.json();
  } catch {
    return json({ detail: "invalid JSON body" }, 422);
  }
  const name = body.name ?? "";
  if (!/^[a-zA-Z0-9_-]{1,64}$/.test(name)) return json({ detail: "invalid agent name" }, 422);
  mkdirSync(DATA_DIR, { recursive: true });
  writeFileSync(MAIN_AGENT_PATH, JSON.stringify({ name }, null, 2) + "\n");
  broadcast(statusPayload());
  return json({ name });
}
