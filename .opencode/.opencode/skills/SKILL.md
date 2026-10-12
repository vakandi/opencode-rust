---
name: job-hunter-weekly-recap
description: Weekly recap + Discord self-audit for the job-hunter-ch-lu subworker via the omp-server API (:5677). The hunter ITSELF reads its workspace, checks its OWN Discord posts, and backfills gaps ITSELF — you only trigger, poll, and verify. Use this whenever the user asks for a weekly recap of the job hunt, wants to check the hunter posted everything to Discord, says "weekly recap", "récap de la semaine", "audit discord du hunter", "did the hunter post everything", or wants opportunities/applications verified against the Discord channels.
---

# Job Hunter Weekly Recap + Discord Self-Audit

The hunter does ALL the work itself: recap from its own files, audit of its own
Discord posts with its own tools, backfill of gaps with its own posts. You NEVER
write the recap yourself, NEVER post to Discord yourself, NEVER summarize from
files and call it a recap. If the agent's session message doesn't contain the
per-item table, the skill FAILED — say so, don't invent it.

## Prerequisites

- omp-server wrapper on `127.0.0.1:5677` (container `omp-server-omp-server-1`).
- Token: `grep '^ELIA_AUTH_TOKEN=' /Users/vakandi/Documents/omp-server/omp-server/.env | cut -d= -f2-`

## Workflow

### Step 1 — Fire via the omp-subworker-trigger skill

Load and follow `.omp/skills/omp-subworker-trigger/SKILL.md`
(`/Users/vakandi/Documents/omp-server/.omp/skills/omp-subworker-trigger/SKILL.md`)
with `<AGENT>` = `job-hunter-ch-lu` and the trigger prompt below VERBATIM.
That skill owns triggering + session-id handoff; this skill owns what to ask and how to verify.
Do NOT reimplement the trigger — use the skill.

Trigger prompt (orders the agent to act itself — read, check, post, then report):

```bash
T="$(grep '^ELIA_AUTH_TOKEN=' /Users/vakandi/Documents/omp-server/omp-server/.env | cut -d= -f2-)"
python3 - "$T" <<'EOF'
import sys, json, urllib.request
token = sys.argv[1]
prompt = (
"SELF-AUDIT RUN — you do everything below YOURSELF with YOUR tools. Do NOT delegate, "
"do NOT ask questions, do NOT summarize without acting. "
"STEP 1 (recap): read YOUR workspace/HANDOFF_NEXT_SESSION.md and YOUR workspace/20*/verify_*.md "
"from the last 7 days. Count: runs completed, opportunities found, applications sent (with dates), failures/errors. "
"STEP 2 (audit, YOUR tools): for EACH item from step 1, check YOUR private Discord channels YOURSELF "
"with discord_send_message/messages.list (guild 1489242790444662885 — opportunities 1553147049426427945, "
"applications 1553147052807037119, reports 1553147056837894335). A handoff line is NOT proof of posting — "
"only a message YOU find in the channel counts as posted. "
"STEP 3 (backfill, YOUR posts): post EVERY missing item YOURSELF right now in the correct card format "
"(opportunity/application/summary per YOUR persona). Skip nothing. An item you cannot verify stays "
"marked UNVERIFIED, never assumed posted. "
"STEP 4 (report): reply with the recap counts AND one table row per item: item | posted YES/NO | "
"backfilled YES/NO/UNVERIFIED. Under 40 lines total. If you posted nothing because everything was "
"already there, say so with the table proving it.")
body = json.dumps({"prompt": prompt}).encode()
req = urllib.request.Request("http://127.0.0.1:5677/trigger/job-hunter-ch-lu", data=body,
    headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
r = urllib.request.urlopen(req, timeout=30).read().decode()
print(r[:300])
open('/tmp/jh_recap_session.txt','w').write(json.loads(r)['session_id'])
EOF
```

Expect `{"status":"triggered",...,"session_id":"ses_…"}` in under 100ms. Save the session id.

### Step 2 — Poll until the run COMPLETES (MANDATORY)

Do NOT report anything before the agent's own reply exists. NEVER use a blind
`sleep N` — the trigger prompt itself is stored as the session's first message,
so "a message exists" or "text longer than N chars" is NOT completion (this exact
mistake fired DONE on the prompt echo before). Completion = ALL of:

1. `omp-server/data/runs.json` shows YOUR `session_id` with `status: completed`
   (or `failed` — report failure, don't wait forever), AND
2. the session's LAST message has `role: assistant` AND its text differs from
   your trigger prompt.

Poll loop (90s, up to 30 min), save as a file and run with `python3 -u`:

```bash
T="$(grep '^ELIA_AUTH_TOKEN=' /Users/vakandi/Documents/omp-server/omp-server/.env | cut -d= -f2-)"
SES=$(cat /tmp/omp_trigger_session.txt)
python3 - "$T" "$SES" <<'EOF'
import sys, json, urllib.request, time
token, want = sys.argv[1], sys.argv[2]
base = "http://127.0.0.1:5677"
def get(path):
    req = urllib.request.Request(base + path, headers={"Authorization": f"Bearer {token}"})
    return json.loads(urllib.request.urlopen(req, timeout=20).read().decode())
for i in range(20):
    ses = get(f"/sessions/job-hunter-ch-lu?limit=5")
    msgs = ses.get('messages', []) if ses.get('session_id') == want else []
    assistant = [m for m in msgs if m.get('info', {}).get('role') == 'assistant']
    last_txt = ''.join(p.get('text','') for p in assistant[-1].get('parts', [])) if assistant else ''
    print(f"poll {i}: msgs={len(msgs)} assistant={len(assistant)} chars={len(last_txt)}", flush=True)
    if assistant and len(last_txt) > 100 and 'SELF-AUDIT RUN' not in last_txt:
        print("DONE", flush=True)
        break
    time.sleep(90)
else:
    print("TIMEOUT — run did not finish, report FAILURE", flush=True)
EOF
```

Then read the agent's LAST assistant message (the table must be in it):

```bash
T="$(grep '^ELIA_AUTH_TOKEN=' /Users/vakandi/Documents/omp-server/omp-server/.env | cut -d= -f2-)"
SES=$(cat /tmp/jh_recap_session.txt)
for i in $(seq 1 20); do
  OUT=$(curl -s -H "Authorization: Bearer $T" "http://127.0.0.1:5677/sessions/job-hunter-ch-lu?limit=5")
  echo "$OUT" | python3 -c "
import sys,json
d=json.load(sys.stdin)
mine=[m for m in d.get('messages',[]) if True]
print('messages so far:', len(mine))"
  sleep 90
done
```

Then read the agent's LAST message (the table must be in it):

```bash
curl -s -H "Authorization: Bearer $T" "http://127.0.0.1:5677/sessions/job-hunter-ch-lu?limit=1" \
| python3 -c "import sys,json; d=json.load(sys.stdin); m=d['messages'][-1]; print('\n'.join(p.get('text','') for p in m['parts']))"
```

### Step 3 — Independently verify the backfill (MANDATORY)

Trust the table only after YOU check the channels show the posts:

```bash
for CH in 1553147049426427945 1553147052807037119 1553147056837894335; do
  mcp-cli call discord-server-mcp discord_execute "{\"operation\":\"messages.list\",\"params\":{\"channel_id\":\"$CH\",\"limit\":10}}" 2>/dev/null | head -c 600; echo
done
```

Every item the agent claims backfilled must appear here. Mismatch = report the gap, don't cover it.

### Step 4 — Report to the user

Relay the agent's counts + table + your verification result (channels confirm Y/N).
If the agent never produced the table: report FAILURE, never a home-made recap.

## Rules

- The recap and table come ONLY from the agent's session message. Anything else is fabrication.
- Read-only except the trigger + the agent's own backfill posts. Never post to Discord yourself.
- Never call `POST /test/frames/job-hunter-ch-lu` (synthetic frames pollute evidence).
