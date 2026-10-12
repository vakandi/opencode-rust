/** WS broadcast hub — TopBar-facing event names only.
 * Handled by SubworkerManager.handleWSMessage: initial_status, status_update,
 * subworker_started, subworker_completed, subworker_success, subworker_cancelled,
 * subworker_error, run_log (+field), run_banner, pong. Anything else is ignored.
 */
import type { ServerWebSocket } from "bun";

export type WsData = { authed: boolean };

const clients = new Set<ServerWebSocket<WsData>>();

export function addClient(ws: ServerWebSocket<WsData>): void {
  clients.add(ws);
}

export function removeClient(ws: ServerWebSocket<WsData>): void {
  clients.delete(ws);
}

export function clientCount(): number {
  return clients.size;
}

export function broadcast(payload: Record<string, unknown>): void {
  if (clients.size === 0) return;
  const text = JSON.stringify(payload);
  for (const ws of [...clients]) {
    try {
      ws.send(text);
    } catch {
      clients.delete(ws);
    }
  }
}

export function emitRunLog(name: string, field: "text" | "reasoning" | "tool", text: string): void {
  broadcast({ event: "run_log", name, text, field });
}

export function emitRunBanner(name: string, banner: { kind: string; error?: string; delaySeconds?: number }): void {
  broadcast({ event: "run_banner", name, banner });
}

export function emitSubworkerStarted(name: string): void {
  broadcast({ event: "subworker_started", name });
}

export function emitSubworkerCompleted(name: string, status: string, sessionId?: string): void {
  const event = status === "completed" ? "subworker_completed" : status === "cancelled" ? "subworker_cancelled" : "subworker_error";
  broadcast({ event, name, status, session_id: sessionId });
}
