# CROSS-ATTACK — ts-anatomy vs perf-architect and pragmatic-architect

Round 2. Targets: **perf-architect**, **pragmatic-architect**. I do not defend my own report except where I
retract it (§A.0). Everything below is measured or `file:line`.

**My assignment, answered up front:** *is the single synchronous `node:sqlite` handle a bigger constraint than
either claims, and is it language-attributable?* — **Yes, and perf-architect never mentions it once in 575
lines. No, it is not language-attributable.** Detail in §A.6 and §D.

---

# A) CLAIMS I REJECT

## A.0 🔴 MY OWN ERROR — RETRACTION (I have to start here, because I over-claimed the same way)

**ts-anatomy Round 1, §9.4:** *"`packages/opencode/src/project/*` … **ABSENT** (no project crate …
`ProjectV2` DTO exists … but no identity resolution)"* and *"**`x-opencode-directory` handling was NOT found** …
The Rust `ServerState` looks single-tenant."*

**This is FALSE.** `crates/opencode-server/src/routes.rs:58`:
```rust
.nest("/session", session_routes().layer(middleware::from_fn(inject_opencode_directory)))
```
`routes.rs:74-86`:
```rust
/// Injected by middleware from x-opencode-directory header for session create.
#[derive(Clone)]
struct OpenCodeDirectory(pub Option<String>);
async fn inject_opencode_directory(mut request: Request<axum::body::Body>, next: Next) -> Response {
    let dir = request.headers().get("x-opencode-directory")
        .and_then(|v| v.to_str().ok()).map(std::string::ToString::to_string);
    request.extensions_mut().insert(OpenCodeDirectory(dir));
    next.run(request).await
}
```
**Why I got it wrong:** my delegated Rust survey only scanned `crates/*/src` for `pub` items. I inferred the
absence of an *instance crate* to the absence of the *feature*. The feature exists, as axum middleware.

**But the outcome of my claim survives, for a reason I did not know:** `OpenCodeDirectory` is consumed at
**exactly one place** — `routes.rs:390`, inside `create_session` (`routes.rs:388-412`). And `session_status`
(`routes.rs:306-308`) takes **no** `Extension<OpenCodeDirectory>`; it reads a process-global
`SESSION_RUN_STATUS` and the global `state.sessions`. So the header is **parsed and scoped to nothing** except
the session-create call. Missing versus TS: `?directory=` precedence, `process.cwd()` fallback, the session-row
override (TS `workspace-routing.ts:182`), `decodeURIComponent`, `InstanceStore`, `InstanceState`, and
`InstanceRef` propagation. **Correct conclusion, wrong mechanism — I got the answer right by asserting a
stronger falsehood than the evidence supported.** That is the actual defect, and it is the same defect I am
about to charge perf-architect with.

**Second, smaller omission:** my §9.1 LOC accounting said 208 `.rs` / 89,698 LOC and implied the plugin host is
pure Rust-plus-shell. I missed `crates/opencode-plugin/host/plugin-host.ts` — **20,233 bytes of TypeScript
shipped inside the Rust crate** — plus `crates/opencode-plugin/builtin/`, `crates/opencode-server/static/`,
`crates/opencode-command/commands/`. The "no embedded JS engine" conclusion still holds (this is a host-side
shim driving an external runtime), but my crate inventory was incomplete.

## A.1 🔴 perf-architect §0 — *"MEASURED BASELINE — this is your live production database"*

**REJECTED. Wrong file.** They measured `~/.local/share/opencode/opencode.db`. That is the **host TUI** DB.

Measured just now, same machine:

| | host TUI DB | container DB |
|---|---|---|
| path | `~/.local/share/opencode/opencode.db` | `EliaAI/subworkers/server/opencode-data/opencode.db` |
| **inode** | **125044873** | **148029286** |
| size | 18,996,281,344 B = **17.69 GiB** | 1,405,136,896 B = **1.31 GiB** |
| mtime | Oct 5 00:27 (**live**) | Oct 5 00:27 |
| who writes it | host PID **65460** `opencode`, 488 MiB RSS, 132.8% CPU, 51 min CPU | bind-mounted container volume |

`SUBWORKERS_SYSTEM.md:671` states the separation explicitly: *"Container opencode DB
(`/root/.local/share/opencode/opencode.db` inside `elia-subworker-srv`) is **bind-mounted** to host
`~/EliaAI/subworkers/server/opencode-data/opencode.db` — **separate file from host TUI DB**
(`~/.local/share/opencode/opencode.db` 300M, 40k sessions)."* `docker-compose.yml:43` confirms the mount.

**Their number is not fabricated.** I re-ran `dbstat` on the host DB and it reproduces:
`event 14.95 GiB · part 1.56 GiB · message 0.87 GiB`. So the analysis is *internally correct for the host TUI
database* and **completely irrelevant to the 8 GB container.** Different inode, different file, different
workload (40k interactive TUI sessions vs ~11 batch subworker agents).

**Consequence:** §0's entire baseline table, the 84.5% headline, "11,409 bytes of DB per published event",
"1,405,820 rows", the part-type distributions, "max `part` row 4.6 MB", "largest session 83.5 MiB" — all of it
is measured on the wrong database. The code they cite (`schema/src/v1/session.ts:502-507`, `core/src/event.ts:337`)
is correct and I confirm it independently; the *magnitude* is attached to the wrong file.

## A.2 🔴 perf-architect's headline — *"the append-only `event` table … is ~95% of the memory pressure"*

**REJECTED as unestablished for the target system.** On the container's actual DB, measured read-only:

```
page_size=4096  page_count=343051  freelist=0     (1.31 GiB total)
part                                0.397 GiB   ← largest object
event_aggregate_type_seq_idx        24.0 MiB
event_aggregate_seq_idx             15.8 MiB
event_sequence                       0.0 MiB  (12 rows)
event                                0.0 MiB  (1 page = 4096 B)
```

Two things follow, and both are fatal to the claim:

1. **The largest object is `part` (0.397 GiB, ~30%), not `event`.** The three event *indexes* total ~40 MiB.
2. **The `event` table is 1 page (4,096 bytes) while its own indexes claim 40 MiB.** An index cannot exceed its
   table. The file is **structurally corrupt** — see §A.3. So the container's event-table share is not
   "84.5%"; it is **not measurable at all**.

You cannot conclude "~95% of memory pressure" from a table you cannot read, in a database nobody has ever
measured on the machine where the stall happens.

## A.3 🔴 NEW FINDING — the container's database is CORRUPT, and has been before

`sqlite3` on `server/opencode-data/opencode.db`, read-only:

```
pragma quick_check;
*** in database main ***
Tree 60 page 60:     btreeInitPage() returns error code 11
Tree 25 page 341972: btreeInitPage() returns error code 11
Tree 25 page 341963: btreeInitPage() returns error code 11
...
select count(*) from part;      → Error: database disk image is malformed (11)
select count(*) from message;   → Error: database disk image is malformed (11)
select count(*) from event;     → Error: database disk image is malformed (11)
select count(*) from session;   → 1019      ← readable
select count(*) from todo;      → 3480      ← readable
select count(*) from event_sequence; → 1021 ← readable
```

The same directory holds the scar tissue: `opencode.db.corrupt` (954,097,664 B, **Sep 15**) and
`opencode.fixed` (1,349,709,824 B, **Sep 23**). Two prior corruption incidents on this exact deployment, and the
live file is malformed today.

**Neither perf-architect nor pragmatic-architect mentions this.** It is the single most actionable fact in this
cross-attack, because it is a *plausible consequence* of the architecture I documented in Round 1 §7.1/§1.8:

- one `DatabaseSync` handle, `Semaphore.make(1)` — `core/src/database/sqlite.node.ts:110`
- `statement.all()` synchronous on the JS main thread — `:63`
- `PRAGMA busy_timeout=5000`, `synchronous=NORMAL`, WAL, **no `wal_autocheckpoint` / `temp_store` tuning** —
  `core/src/database/database.ts:27-32`
- **one `BEGIN IMMEDIATE` transaction per durable part/message event, ~35-55 write statements per assistant
  turn** — `core/src/event.ts:237-363`, `core/src/session/projector.ts:310-328`

A single serialized writer issuing immediate transactions against one file is a textbook page/WAL corruption
workload. **I am not claiming that is the cause** — I have not proven causation, and two prior incidents could
have been disk or Docker-layer failures. But the architecture supplies no defence, and the failure mode is
observed twice on this deployment.

## A.4 🔴 perf-architect §3.1 — *"The SSE fan-out is NOT your bottleneck … correctly bounded … Do not let anyone prioritise this."*

**REJECTED on both halves. Both halves are wrong.**

**(a) The bound they cite does not exist.** They wrote
*"`handlers/event.ts:11` `subscriberCapacity = 256` and `core/event.ts:154` `Queue.dropping(256)`"* and called it
correctly bounded, instructing the team not to prioritise it. Measured:

- `handlers/event.ts` — **no `subscriberCapacity` anywhere in the file.** The per-connection queue is
  `Queue.unbounded<EventV2.Payload>()` (~line 31), and it is fed from `events.listen` — the *unbounded*
  `listeners[]` array (`core/src/event.ts:606-613`). Events are offered into the queue at `:32` and filtered by
  directory **afterwards** at `:35-38`.
- `core/event.ts:154` `Queue.dropping<Payload, SubscriberOverflowError>(capacity)` **does** exist — it is the
  opt-in helper `allBounded`. **The `/event` route does not use it.**
- Their own §3.2 admits `core/event.ts:175` is `PubSub.unbounded` for `pubsub.all`. So they had the evidence in
  their own report and concluded the opposite.

**(b) There IS a production subscriber — they asserted zero.** They wrote *"your `opencode_client.py` never opens
the SSE `/event` stream"* and *"With 0 production subscribers this costs nothing"* and *"Your Rust SSE can be
best-effort with zero retention, because no production consumer subscribes."*

`opencode_client.py` is indeed pure-HTTP — **they are right about that file**. But the production consumer is
`app/services/runner.py`, and it opens an SSE stream on every run:

```
app/services/runner.py:637:  url = f"{server_url}/event?directory={quote(directory)}"
```
feeding a per-field delta dedup emitter (`:640-648`) that bridges into `app/routes/websocket.ws_manager`
(`:633`). So:

- **Unbounded queues are live**, once per subworker run, and a stalled websocket consumer grows them without
  limit.
- Worse, it is filtered on `event.location?.directory === instance.directory`. Per `runner.py:431-443` the
  directory is a `/data` → `$OPENCODE_WORKSPACE` rewritten path. **A directory mismatch silently drops 100% of
  events** — and per my Round 1 §1.1, `?directory=` query beats the `x-opencode-directory` header
  (`workspace-routing.ts:87`), while an existing session's stored `directory` overrides both (`:182`).

**Their recommendation — "Rust SSE can be zero-retention" — would break the operator's live view.** That is a
behavioural regression proposed on the strength of a subscriber count that was measured on the wrong file.

## A.5 perf-architect §3 — *"`GET /session/status` — **full status map for all sessions**"*

**REJECTED.** `packages/opencode/src/session/status.ts:39-48`:

```ts
const set = Effect.fn("SessionStatus.set")(function* (sessionID, status) {
  ...
  yield* events.publish(Event.Status, { sessionID, status })
  if (status.type === "idle") {
    yield* events.publish(Event.Idle, { sessionID })
    data.delete(sessionID)      // ← idle sessions are REMOVED, not stored as {"type":"idle"}
    return
  }
  data.set(sessionID, status)
})
```

The map is **busy/retry-only**. This is not pedantry: it is load-bearing for their §3 conclusion that polling is
cheap, and for the whole team's understanding of completion detection. The consumer's `wait_for_response`
works *by absence* (`opencode_client.py:240-241` supplies the `{"type":"idle"}` default). Their mental model of
this endpoint is wrong in a way that hides a real consumer bug — see §C.3.

## A.6 🔴 perf-architect §1b — the synchronous handle is a bigger constraint than they claim, and they never mention it

They rated the per-step full-history re-read **"Not the problem"** (`:136`) after measuring ~5.5 MB typical
peak per active turn, and their §9 CPU section never mentions the driver.

**Their RAM arithmetic is sound and I do not dispute it.** Their citations check out: `prompt.ts:1093`,
`message-v2.ts:473-494`, `hydrate()` `:97-120`, `filterCompacted` `:525-575`, `native-runtime.ts:107`
(`Queue.unbounded<LLMEvent, Cause.Done>()`) and `:126` (`FiberSet.run`) — all verified.

**But they optimised the wrong resource.** The constraint is not heap, it is **the single serialized I/O
path**, and it is language-specific:

- Every message/part read and write goes through **one** `Semaphore.make(1)` permit
  (`core/src/database/sqlite.node.ts:110`) onto **one** `DatabaseSync` (`:151-157`) whose every call is
  `statement.all(...)` — **synchronous C on the JS main thread** (`:63`). There is no worker thread, no pool,
  no `node:worker_threads`.
- Therefore the per-step full-history re-read they measured as "5.5 MB and fine" is, in **time**, a full
  serialized scan of the session's entire message + part history **per LLM step** — `⌈N/50⌉` message queries plus
  `⌈N/50⌉` part queries (`message-v2.ts:473-494`, `:429-470`), each JSON-parsing every row.
- And the write side they measured as "~35-55 statements" (§A.1's mechanism, which they got right) contends for
  **that same single permit**, while **blocking the event loop**.

So the correct reading of their own §1b is: *not a RAM problem, but a throughput problem that scales with
session length × steps × concurrent sessions, and it is serialized globally.* In an 8 GB box the symptom would
be **slowness and timeouts**, not OOM — which is arguably a better match to "works then suddenly stalls" than
the RSS-ratchet story they tell at §1c.

**And this is the part they omitted entirely:** grep their 575 lines for `Semaphore`, `DatabaseSync`,
`sqlite.node`, `statement.all`, `busy_timeout`, `synchronous` — **zero hits.** The single most important
structural fact about the persistence layer, in a report whose thesis is about persistence, is absent.

## A.7 🔴 pragmatic-architect §1.6 "F6 — confirmed API gaps vs the consumer" — grossly under-scoped

**Not gaps. Three core endpoints are wire-incompatible.** Their §1.6 found shape differences; they did not find
that the consumer cannot drive this server at all. Measured against `opencode_client.py`:

**(a) `POST /session/{id}/message` — fails on every call.**
`routes.rs:710-715`:
```rust
pub struct SendMessageRequest {
    pub content: String,            // ← REQUIRED, no #[serde(default)]
    pub model: Option<String>,     // ← String
    pub variant: Option<String>,
    pub stream: Option<bool>,
}
```
Consumer sends (`opencode_client.py:367-374`):
```python
payload = {"parts": [{"type":"text","text":content}], "agent": agent,
           "model": {"modelID": ..., "providerID": ...}, "variant": variant}
```
`parts` does not exist in the DTO. `content` is a required `String` with no default ⇒ **deserialization fails,
every message**. `model` is `Option<String>` but receives an **object**. `agent` does not exist.
`grep -c deny_unknown_fields routes.rs` → **0**, so even the survivable fields are silently dropped.

**(b) `POST /session` — the agent is hardcoded.**
`routes.rs:370-374` `CreateSessionRequest { parent_id, title, permission }` — **no `agent` field.** And the
handler: `state.sessions.lock().await.create("default", directory)`. The consumer sends `agent`
(`opencode_client.py:262`). It is **silently discarded**. Every subworker would run as the literal agent
`"default"` instead of `gilfoyle`/`setbon`/etc. — the exact failure `opencode_client.py:362-363` warns about.
Note `runner.py:455` logs `response_agent=sess.get("agent","unknown")`, so this **would** have been visible in
the logs, had anyone run it.

**(c) `GET /session/status` — `wait_for_response` can never succeed.**
`routes.rs:357-367`:
```rust
pub struct SessionStatusInfo {
    pub status: String,   // "busy" | "retry" | lifecycle name
    pub idle: bool,
    pub busy: bool,
    pub attempt: Option<u32>, pub message: Option<String>, pub next: Option<i64>,
}
```
**There is no `type` field.** Consumer: `status.get("type") == "busy"` (`:544`), `== "idle"` (`:558`).
Both are `None` forever ⇒ Phase 1 exhausts its deadline and raises
`OpenCodeTimeoutError("Session … never became busy")` (`:549`) ⇒ **every single run fails.**
Also the vocabulary differs: Rust `SessionStatus` is `Active | Completed | Archived | Compacting` — **no `Idle`,
no `Busy`**; idle/busy are separate booleans (`routes.rs:315-330`).

**(d) `GET /session/{id}/message` — returns the wrong envelope.**
`routes.rs:1098-1113` returns `Vec<MessageInfo>`; `MessageInfo` is
`{id, session_id, role, parts, created_at, completed_at}` (`:718-724`) — **flat**. The consumer reads
`msg.get("info", {})` then `info.get("role")` (`:449-452`), and `extract_assistant_text` checks
`info.get("role") != "assistant"` (`:618`). With a flat shape, `info` is always `{}` ⇒
`user_count = assistant_count = 0`, `has_text = False`, and **`extract_assistant_text` returns `""` for every
run** — so every subworker would post an empty report even if the rest worked.

## A.8 🔴 pragmatic-architect §3 — committed to **(B) INCREMENTAL PORT using `/tmp/OpenCodeRust` as the base**

**REJECTED as unsound, on §A.7.** A base whose `POST /message` cannot deserialize the client's payload, whose
`POST /session` discards the agent, whose `/session/status` has no `type` field, and whose message list has the
wrong envelope, is not a base for the consumer contract. Their §4 "NON-NEGOTIABLE (missing any ⇒ the port is
useless)" list cannot be satisfied by incrementally adopting this base — the four items above are exactly the
non-negotiables, and all four are broken.

The error is one of **category**: they audited `/tmp/OpenCodeRust` as *an implementation of opencode* and found
it incomplete. They did not audit it as *a wire-compatible server for this specific client*, which is the only
question that matters for adoption. A base can be 70% feature-complete and 0% adoptable.

## A.9 pragmatic-architect §1.3 — *"In a supervised fleet that is a restart loop."*

**REJECTED for this deployment — Docker semantics.** `restart: unless-stopped`
(`docker-compose.yml:5`) triggers on **process exit**, *not* on healthcheck failure. Healthcheck
(`docker-compose.yml:84-89`): `interval: 10s`, `timeout: 5s`, **`retries: 3`**, **`start_period: 30s`**.

- `start_period: 30s` suppresses failure counting for the first 30 s; the container stays `starting`.
- After that, 3 failures at 10 s intervals ⇒ the container is marked `unhealthy` at roughly **t+60 s**.
- **Unhealthy does not trigger `restart:`.** With opencode binding at t+70-90 s (their measurement), the process
  never exits, nothing restarts, and the server becomes healthy on the next probe.

So the 70-90 s cold start is **not** a restart loop here. The genuine risk is narrower and different: the
**Python scheduler's own health gate** during startup. Note also `docker-compose.yml:7` — the entrypoint has
its *own* supervisor loop for OOM-kills, which is a different mechanism from Docker restarts and would not
help a bind that is merely late.

Their §1.3a bun/deno root-cause, by contrast, is **excellent and I confirm its consequence**: if
`detect_runtime()` (`plugin/subprocess/runtime.rs:52`) prefers bun and bun hangs before `initialize`, then the
host has **never completed a handshake with any plugin** — so the 13-hook allowlist gap they cite is measured
against a code path that has never run. That is a sharper framing than they gave it.

## A.10 perf-architect §2 — *directory fan-out ~11×*

**PARTIALLY REJECT — right conclusion, wrong citation, and one under-count.** They cite
`SUBWORKERS_SYSTEM.md:53` (`/tmp/{name}`) as the directory scheme. That is **stale**. Actual code,
`app/services/runner.py:431-443`: `directory` = `$OPENCODE_WORKSPACE`-rewritten, defaulting to `/data` for the
main agent and `/data/subworkers/{agent_id}/workspace` otherwise; `docker-compose.yml:17` sets
`OPENCODE_WORKSPACE=/data`. So the fan-out is `1 + (enabled non-main agents)` distinct directory keys —
**their "~11" ballpark is right**, and I withdraw the challenge to the multiplier. I flag only that the
mechanism is per-agent workspace directories, not `/tmp/{name}` scratch dirs, which matters for whether a fix
is "one shared instance" or "per-agent".

**Their supporting claim that `dispose_instance` has zero callers is confirmed**: grep over `app/` finds only
the definition (`opencode_client.py:313`) — the two call sites are `abort_session` (`runner.py:224`, `:798`),
never dispose. And their `InstanceState.make` site count is **23** non-test, not 22 (immaterial).
Their `mcp/index.ts:492` and `lsp/lsp.ts:145` citations are correct.

---

# B) CONTRADICTIONS

**B.1 — SSE: three-way, and everyone who touched it was wrong.**
- perf-architect: *"no production consumer subscribes … correctly bounded … do not prioritise."*
- ts-anatomy Round 1 §7.5 #8: flagged `Queue.unbounded()` per connection as a risk (right mechanism, I did
  not know about `runner.py:637`).
- Measured: the bound perf-architect cited does not exist, **and** `runner.py:637` opens `/event` per run.
**perf-architect loses on both halves.** My Round 1 was directionally right but I never checked whether a
subscriber existed — also a failure I should own.

**B.2 — Both members claim to have located *the* memory mechanism, in different processes, neither measured on
the target.**
- perf-architect: 17.68 GiB DB, `event` = 84.5%, ~95% of pressure — **host TUI DB**.
- pragmatic-architect F2: `SessionManager.events` never drained + whole-table resync = "the 8GB failure mode"
  — **the Rust reference's** in-memory design.
- pragmatic-architect §10(their own): says language is not the bottleneck.
- Measured: the 8 GB container (`elia-subworker-srv`) **is not running** — `docker ps -a --filter
  name=elia-subworker-srv` returns **empty**, not even a stopped container. So *neither* number was ever
  taken on the system the 20-session stall occurs in. They are answering "what will hurt us if we adopt Rust"
  and "what hurt the host TUI", then both being read as "what hurts the subworker container."

**B.3 — perf-architect says the language is not the bottleneck; pragmatic-architect's own F3 says swapping to
Rust makes availability strictly worse.** 70-90 s to bind, on a deployment whose healthcheck has ~60 s of grace
and whose client is a synchronous `httpx` caller with a 30 s `send_message` timeout
(`opencode_client.py:342`). Nobody reconciled these. Either F3 is discounted (and it should be — see §A.9,
it's a two-line fix) or "Rust is faster" has a real availability cost that must be priced.

**B.4 — perf-architect and I disagree on `event` retention.** They say *"nobody reads it"*, citing
`handlers/sync.ts:51` + `control-plane/workspace.ts:351,402`. **I verified this is correct** — those are the only
callers of `replay`/`replayAll`/`subscribeDurable` outside their own definitions. So on the subworker
deployment (no sync) it *is* write-only amplification. **This is a point where the cross-attack fails to land:
their code reading is better than mine and I say so.** My Round 1 §7.4 said "no retention" and did not prove the
"never read" half. They did.

---

# C) SURVIVING CLAIMS

Now load-bearing after cross-attack.

**C.1 — The architecture is language-attributable only in its I/O layer, and that layer is Rust's best win.**
The single `Semaphore.make(1)` + synchronous `DatabaseSync` (`sqlite.node.ts:110`, `:63`) is the one constraint
where the rewrite is unambiguously worth it: `sqlx`/`rusqlite` give a real pool, non-blocking I/O, a settable
`busy_timeout`, `BEGIN IMMEDIATE` retry loops, and checkpoint control. Everything else perf-architect measured
is architecture, not language. **Verdict: the rewrite is justified by §A.6 alone — and §A.6 is not the reason
perf-architect gave.**

**C.2 — The `event` log is genuinely write-only on this deployment, and genuinely unpruned.** perf-architect's
"never read" is verified (§B.4). My §7.4 "no `DELETE` on `event` anywhere" is verified. Even though both
measured the wrong *file*, the *code* claim transfers — same schema, same code path. And
`/tmp/OpenCodeRust` has **no `event` table at all** (`storage/schema.rs` has 6 tables), so the port drops it by
construction. **This is a case where the Rust reference is genuinely better, and pragmatic-architect should be
given credit: §1.2 F2's "unbounded `events` Vec never drained" is the *same disease* the TS has in a different
shape — TS spills it to a 15 GiB append-only table, Rust spills it to RAM. Neither is acceptable; the Rust one
is strictly worse per byte.**

**C.3 — The consumer's completion detection is broken on TS *and* unimplementable on Rust.** TS: idle is deleted
(`status.ts:42-46`), status is `InstanceState` keyed by directory (`:26-28`), and the client calls
`get_session_status` with **no directory** (`opencode_client.py:543`, `:557`) ⇒ falls back to `process.cwd()`.
Rust: no `type` field at all (§A.7c). **There is no third option.** Any port must *change the client*, which is
outside the 25-agent blast radius the team has been treating as fixed.

**C.4 — Per-directory isolation is the real fan-out, and neither design fixes it.** TS: 23 non-test unbounded
`InstanceState.make` sites (verified; perf-architect said 22), keyed by directory, with no TTL/LRU, and
`dispose_instance` has **zero callers** (verified). The 1 + N directory multiplier is real (§A.10).
Rust: header parsed, scoped to one handler (§A.0). **Both leak; neither bounds.**

**C.5 — The container DB corruption (§A.3) is real, reproducible, and unattributed.** Nobody else found it. It
must be root-caused before any port decision, because "port it and the problem goes away" is unanswerable while
the current system's own database is unreadable.

**C.6 — `/tmp/OpenCodeRust` has 330 lines of tests and no CI (pragmatic-architect §1.4/§1.5).** I independently
measured the same 89,698 LOC base. Confirmed and load-bearing: combined with §A.7, adoption cost is far higher
than "incremental."

**C.7 — The plugin host has never completed a handshake (pragmatic-architect §1.3a).** This is the strongest
argument in any of the three reports, and it is *strengthened* by my own §8 finding that TS's `Plugin.trigger`
awaits every plugin **sequentially** (`plugin/index.ts:284-297`) and holds them **per-directory**. Two
independent analyses converging on "the plugin layer is the least-verified, highest-risk surface in both
implementations" is a strong signal.

---

# D) THE ONE THING THAT MATTERS MOST

**We have never measured the system that stalls, and two of the three candidate root causes are therefore
unfalsified — while the two facts we *can* state are that its database is corrupt and that the client contract
is unimplementable on the proposed target.**

Concretely, in priority order:

1. **`elia-subworker-srv` does not exist** (`docker ps -a` → empty). Every memory number in this round is from
   `vouch-opencode` (799 MiB / 1.172 GiB — a different container) or the host TUI DB (17.69 GiB — a different
   inode). **There is no baseline.** perf-architect's arithmetic is internally consistent and externally
   untethered.
2. **The container's real DB is 1.31 GiB, structurally corrupt, `part` is the largest object at 0.397 GiB, and
   `event` is unreadable.** So the "84.5% event ⇒ 95% of pressure" thesis is not merely misfiled — it is
   **untestable on the target today**.
3. **`/tmp/OpenCodeRust` cannot serve the client** (§A.7: four wire-level breaks, one of which hardcodes the
   agent to `"default"`). So "adopt it incrementally" is not a cost question, it is a validity question.

**⇒ The decision in front of the team — rewrite language, adopt the Rust base, or fix config — cannot be made
on this evidence, and the cheapest action that would resolve most of it costs minutes and invalidates nothing:
stand up `elia-subworker-srv`, capture one live memory profile (`docker stats` + cgroup `memory.peak` +
`memory.events`), and separately run `.recover`/`integrity_check` on the container DB.** Until someone does
that, every confident number in this round is somebody's arithmetic about a process they did not observe.

**My own answer to my assigned question, stated plainly so it isn't lost:** yes, the single synchronous
`node:sqlite` handle is a larger constraint than perf-architect claims — larger than the DB growth he measured,
because it is a *latency and event-loop* constraint rather than a heap one, and it is the only constraint here
that Rust definitively fixes. **And no, it is not language-attributable in the way the framing implies** — the
transaction storm that feeds it (§C.2) is architecture. A faithful port that kept one-writer-per-event would
reproduce both the throughput cliff and the corruption risk in Rust.


---

# ROUND 3 — CONCEDE, POSITION, PLAN CONSTRAINTS

## 1. CONCESSIONS (complete; I withdraw all)

1. **§0 "legacy v1 is generated SDK types only, not a live router."** **FALSIFIED** (feasibility-skeptic A1) —
   I proved a routing negative with an **auto-generated OpenAPI client** (`sdk/js/src/gen/sdk.gen.ts:1`), which
   emits a method per spec operation regardless of routing. `/session/status` is live
   (`groups/session.ts:121-131`). **Correction: 2 live route trees, not 3 — I conflated SDK artifacts with
   servers.**
2. **§9.4 "`x-opencode-directory` absent / no instance crate."** **FALSIFIED** — `routes.rs:58,78`: inferred
   absence of a crate to absence of a feature. **"No project crate" partly wrong** — `ProjectV2` DTO at
   `routes.rs:3263`.
3. **§9.1 inventory incomplete** — missed `crates/opencode-plugin/host/plugin-host.ts` (20,233 B of TS inside
   the Rust crate), plus `plugin/builtin/`, `server/static/`, `command/commands/`.
4. **§7.5 row 1 is a category error** (perf-architect A7) — ranked a latency constraint #1 on a memory question.
   I corrected this in §A.6 then left §7.5 unamended; my report contradicted itself.
5. **§7.4 "storage gap" understated** — disk 95%, 23 GiB free, 19 GB append-only. **Carrying plugin-host's
   caveat: APFS at 96% still completes writes, no `SQLITE_FULL` handling ⇒ time bomb, not proven cause.**
6. **§0 "v1.18.34" under-defended** — inferred, not probed. Host **1.18.34**; container resolves
   `releases/latest` at build time (`Dockerfile:48-56`) under a **false "pinned, not main"** comment.
7. **bypassAgentCheck under-classified** — verified a permission bypass; plugin-host confirmed independently.
8. **Appendix A deleted `src/plugin/tui/**`** against my own "compile-clean isn't evidence" standard; it removes
   `PluginModule.tui`, which loads `tui-switch-proxy.tsx` (`SolidPlugin`) — sidebar + proxy switcher.
9. **PRAGMA citation** — the block is `core/src/database/database.ts:27-33`, not `core/src/event/sql.ts:12`
   (the drizzle `sqliteTable`). Conclusion unaffected.

**Intact:** §C above, unchanged — notably the durable-event tax (`event.ts:237-363`), the sync-handle
serialization ceiling, and the 4 wire-level breaks vs OpenCodeRust.

## 2. POSITION: **(d)** — I am moving off my Round 3 (c).

Not because I was persuaded, but because **(c) is malformed: it presumes a running system.** "Fix configuration +
corruption in TypeScript" cannot execute against a container that does not exist, whose b-trees are unreadable,
whose version resolves to `releases/latest`. (a) and (b) need that same missing target. **All four options assume
a diagnosable system and an identified artifact; neither exists.** Hence (d): **restore service, pin the artifact,
capture one live baseline, re-decide.**

I still rate the `Semaphore.make(1)` serialization ceiling the strongest argument for a port, and **I never reach
(a)**: `opencode-session` maps module-for-module (14 modules, 1:1), so greenfield discards working structure.
**If a port follows, (b) over (a)** — agreeing with plugin-host.

## 3. WHAT THE PLAN MUST CONTAIN

**Ordering constraint — porting is gated behind all five:**

1. **`bypassAgentCheck` fixed** (`prompt.ts:176,1223`, `tool/task.ts:119`) — **live on all 25 agents**; owner
   independent of the port decision.
2. **Service restored** — `elia-subworker-srv` does not exist.
3. **Artifact pinned** — fingerprint the image, probe endpoints, **fix `Dockerfile:48-56`**. No target till then.
4. **DB recovered** — `opencode.fixed` is verified clean (20,667 messages).
5. **One live memory *and latency* profile** — RSS, `memory.peak`, `memory.events`, turn latency under
   concurrency: per §A.6 the ceiling is latency, not RAM.

**Decisions the plan must force:** client is in scope (4 wire breaks — `SendMessageRequest.content` required,
agent hardcoded `"default"`, no `type` in `SessionStatusInfo`, flat envelope); a **named owner for the plugin
contract** — unowned, can ship 3-of-4 headline features silently dead while health checks pass.

**Confirmed by plugin-host:** `bypassAgentCheck` is exploitable via `do-sql.js` — prompt text `@known-agent`
disables the `task` gate. **#1 correctness defect.**

**Won't let the plan skip:** DB recovery before measurement; client-side changes; a live profile of the real
container; pinning the Dockerfile; naming that plugin owner. **Every memory number to date is arithmetic about a
process nobody observed** (`docker ps -a` empty).
