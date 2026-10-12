/** Synthetic frame test (Phase 2 exit gate): emit text/reasoning/tool run_log
 * frames + one run_banner on a fake session WITHOUT starting an engine.
 * Usage: ELIA_AUTH_TOKEN=... bun omp-server/testFrames.ts [subworker-name]
 * TopBar pointed at :5677 must show all three streams in the run popup.
 */
import { appendFrame, snowflakeSessionId, upsertRun, nowIso } from "./store.ts";
import { emitRunLog, emitRunBanner, emitSubworkerStarted, broadcast } from "./ws.ts";
import { statusPayload } from "./statusView.ts";

const NAME = process.argv[2] ?? "elia";
const sessionId = snowflakeSessionId();
const stamp = nowIso();

upsertRun({ name: NAME, session_id: sessionId, status: "running", created_at: stamp, last_frame_at: stamp, started_at: stamp });
emitSubworkerStarted(NAME);

const frames: Array<["text" | "reasoning" | "tool", string]> = [
  ["text", "synthetic text frame: the engine bridge is streaming correctly."],
  ["reasoning", "synthetic reasoning frame: considering the plan, weighing alternatives, proceeding stepwise."],
  ["tool", "read completed"],
];
for (const [field, text] of frames) {
  appendFrame(sessionId, field, text);
  emitRunLog(NAME, field, text);
  await Bun.sleep(300);
}
emitRunBanner(NAME, { kind: "info", delaySeconds: 0 });
broadcast(statusPayload());
console.log(`synthetic run emitted name=${NAME} session=${sessionId}`);
