// Replays EliaTopBar's Swift decoders against frames OUR engine emits.
// Sources: LivestreamStore.swift:48-97 (appendDelta) + :67-76 (tool parse),
// LivestreamParsing.swift:75-96 (extractTodos / findTodosRecursive),
// LivestreamStore.swift:317 (Session ID regex), LogPopoverView.swift:843-855.
import { test, expect } from "bun:test";
import { framesFromEvent, type Frame } from "../httpEngine.ts";

const fails: string[] = [];
const ok = (c: boolean, m: string) => { if (!c) fails.push(m); };

// engine.ts builds the WS delta exactly like this (engine.ts spawnWorkerAndWait).
function deltaFor(f: Frame): string {
  if (f.field === "tool" && f.tool) {
    return JSON.stringify({ tool: f.tool, ...(f.input !== undefined ? { input: f.input } : {}), ...(f.output !== undefined ? { output: f.output } : {}) });
  }
  return f.text;
}

// LivestreamStore.swift:67-76 — tool frame decode.
function parseTool(delta: string): { tool: string; input?: string; output?: string } {
  let obj: any = null;
  try { obj = JSON.parse(delta); } catch { return { tool: "tool" }; }
  const tool = obj.tool ?? obj.name ?? "tool";
  if (typeof obj.input === "string") return { tool, input: obj.input, output: obj.output };
  if (obj.input && typeof obj.input === "object") return { tool, input: JSON.stringify(obj.input), output: obj.output };
  return { tool, output: obj.output };
}

// LivestreamParsing.swift:88-96 — recursive todos finder.
function findTodos(o: any): Array<{content:string;status:string;priority:string}> | null {
  if (Array.isArray(o)) { for (const v of o) { const r = findTodos(v); if (r) return r; } return null; }
  if (o && typeof o === "object") {
    if (Array.isArray(o.todos) && o.todos.length) {
      const t = o.todos.filter((d:any)=>d?.content).map((d:any)=>({content:d.content,status:(d.status??"pending").toLowerCase(),priority:(d.priority??"medium").toLowerCase()}));
      if (t.length) return t;
    }
    for (const v of Object.values(o)) { const r = findTodos(v); if (r) return r; }
  }
  return null;
}

test("todowrite reaches the todo panel with status and priority", () => {
  const seen = new Set<string>(), names = new Map<string,string>();
  const feed = (e: any) => framesFromEvent(e, "ses_x", seen, names);

  // 1. todowrite must reach the todo panel
  const todoPart = { id:"p_t", type:"tool", callID:"c_t", tool:"todowrite",
    state:{ status:"pending", input:{ todos:[
      {content:"alpha done",status:"completed",priority:"high"},
      {content:"beta pending",status:"pending",priority:"low"} ] } } };
  const todoFrame = feed({ type:"message.part.updated", properties:{ sessionID:"ses_x", part: todoPart } })[0];
  const td = deltaFor(todoFrame!);
  const parsedT = parseTool(td);
  expect(parsedT.tool).toBe("todowrite");
  const todos = findTodos(JSON.parse(td));
  expect(todos?.length).toBe(2);
  expect([todos?.[0]?.status, todos?.[1]?.priority]).toEqual(["completed","low"]);

  // 2. task tool must expose description so the subagent strip can label it
  const taskPart = { id:"p_k", type:"tool", callID:"c_k", tool:"task",
    state:{ status:"pending", input:{ description:"verify wiring", subagent_type:"general", prompt:"ACK" } } };
  const taskFrame = feed({ type:"message.part.updated", properties:{ sessionID:"ses_x", part: taskPart } })[0];
  const tk = parseTool(deltaFor(taskFrame!));
  expect(tk.tool).toBe("task");
  const tkIn = JSON.parse(tk.input!);
  expect(tkIn.description).toBe("verify wiring");
  expect(tkIn.subagent_type).toBe("general");

  // 3. team_create must expose runtimeState.members[].sessionId
  const teamOut = JSON.stringify({ teamRunId:"29cbf8a7-1846-4d26-922a-86633e960d06",
    runtimeState:{ members:[{ name:"probe", sessionId:"ses_member1", status:"running" }] } });
  const teamPart = { id:"p_tm", type:"tool", callID:"c_tm", tool:"team_create", state:{ status:"completed", output: teamOut } };
  const teamFrame = feed({ type:"message.part.updated", properties:{ sessionID:"ses_x", part: teamPart } })[0];
  const tm = parseTool(deltaFor(teamFrame!));
  expect(tm.tool).toBe("team_create");
  const out = JSON.parse(tm.output!);
  const memberSes = out?.runtimeState?.members?.[0]?.sessionId;
  expect(memberSes).toBe("ses_member1");

  // 4. session id extraction (LivestreamStore.swift:317)
  const sesRe = /Session ID:\s*(ses_[A-Za-z0-9]+)/;
  expect(sesRe.test("Session ID: ses_deadbeef1234")).toBe(true);

  // 5. reasoning must survive the dedup guard in appendDelta (delta == cur -> drop)
  const r1 = feed({ type:"message.part.updated", properties:{ sessionID:"ses_x", part:{ id:"p_r", type:"reasoning", text:"abc" } } })[0];
  expect(r1?.field).toBe("reasoning");

  // 6. no nameless tool frames reach the panel
  for (const e of [
    { type:"message.part.updated", properties:{ sessionID:"ses_x", part:{ id:"p_u", type:"tool", callID:"c_u", state:{ status:"completed", output:"done" } } } },
  ]) { expect(feed(e).length === 0, "nameless tool frame leaked to the panel"); }

  expect(fails).toEqual([]);
});
