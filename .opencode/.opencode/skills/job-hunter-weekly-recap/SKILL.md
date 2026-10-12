---
name: job-hunter-weekly-recap
description: Weekly recap + Discord audit for the job-hunter-ch-lu subworker via the omp-server API (:5677). Use this whenever the user asks for a weekly recap of the job hunt, wants to check the hunter posted everything to Discord, says "weekly recap", "récap de la semaine", "audit discord du hunter", "did the hunter post everything", or wants opportunities/applications verified against the Discord channels.
---

# Job Hunter Weekly Recap + Discord Audit

Trigger `job-hunter-ch-lu` on demand for a weekly work recap AND a self-audit of its
Discord reporting (every opportunity, application, failure must have been posted).

## Prerequisites

- omp-server wrapper on `127.0.0.1:5677` (container `omp-server-omp-server-1`).
- Token: `grep '^ELIA_AUTH_TOKEN=' /Users/vakandi/Documents/omp-server/omp-server/.env | cut -d= -f2-`

## Workflow

### Step 1 — Trigger the recap + audit run

```bash
T="$(grep '^ELIA_AUTH_TOKEN=' /Users/vakandi/Documents/omp-server/omp-server/.env | cut -d= -f2-)"
python3 - "$T" <<'EOF'
import sys, json, urllib.request
token = sys.argv[1]
prompt = (
"WEEKLY RECAP + DISCORD AUDIT — do NOT hunt new listings this run. "
"(1) Recap: read workspace/HANDOFF_NEXT_SESSION.md + workspace/20*/verify_*.md from the last 7 days. "
"Report: runs completed, qualifying opportunities found, applications sent (with dates), failures/errors. "
"(2) Audit: for EACH opportunity/application/failure in those files, check the private Discord channels "
"(opportunities 1553147049426427945, applications 1553147052807037119, reports 1553147056837894335, guild 1489242790444662885) "
"via discord_send_message/messages.list — was it posted? "
"(3) Backfill: post anything missing NOW in the correct card format, then report a table: item | posted YES/NO | backfilled YES/NO. "
"Keep the whole reply under 40 lines.")
body = json.dumps({"prompt": prompt}).encode()
req = urllib.request.Request("http://127.0.0.1:5677/trigger/job-hunter-ch-lu", data=body,
    headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
print(urllib.request.urlopen(req, timeout=30).read().decode()[:300])
EOF
```

Expect `{"status":"triggered",...,"session_id":"ses_…"}` in under 100ms.

### Step 2 — Wait and read the result

Runs take 2–10 min (model + MCP probes). Then:

```bash
T="$(grep '^ELIA_AUTH_TOKEN=' /Users/vakandi/Documents/omp-server/omp-server/.env | cut -d= -f2-)"
curl -s -H "Authorization: Bearer $T" "http://127.0.0.1:5677/sessions/job-hunter-ch-lu?limit=1" \
| python3 -c "import sys,json; d=json.load(sys.stdin); m=d['messages'][-1]; print('\n'.join(p.get('text','') for p in m['parts']))"
```

### Step 3 — Report to the user

Summarize: recap counts (runs/opportunities/applications/failures) + audit table
(posted vs backfilled). Flag any item that could NOT be verified. Never claim
"all posted" without the per-item table.

## Rules

- Read-only except the trigger + the agent's own backfill posts. Never post to Discord yourself.
- Never call `POST /test/frames/job-hunter-ch-lu` (synthetic frames pollute evidence).
- If the run fails or empties, say so — don't invent a recap.
