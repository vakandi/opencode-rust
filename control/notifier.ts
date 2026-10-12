/** Discord notifier — mirrors live alert_manager.py + qwen notifier.py.
 * Webhook URL from DISCORD_WEBHOOK_URL env only; never logged.
 * Debounce 300s per (name, kind).
 */
const lastSent = new Map<string, number>();
const DEBOUNCE_MS = 300_000;

function webhookUrl(): string {
  return (process.env.DISCORD_WEBHOOK_URL ?? "").trim();
}

export function notifierEnabled(): boolean {
  return webhookUrl().length > 0;
}

export async function notify(name: string, kind: string, text: string): Promise<void> {
  const url = webhookUrl();
  if (!url) return;
  const key = `${name}:${kind}`;
  const now = Date.now();
  if (now - (lastSent.get(key) ?? 0) < DEBOUNCE_MS) return;
  lastSent.set(key, now);
  try {
    await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ content: `**[omp-server:${kind}]** ${name}: ${text}`.slice(0, 1900) }),
      signal: AbortSignal.timeout(10_000),
    });
  } catch (err) {
    console.error(`[notifier] discord post failed name=${name} kind=${kind} error=${String(err)}`);
  }
}
