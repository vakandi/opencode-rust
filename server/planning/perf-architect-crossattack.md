# PERF-ARCHITECT — Round 2 Cross-Attack

**Member:** perf-architect · **Round 2** · Targets assigned: `plugin-host`, `pragmatic-architect`.
Falsifying: all four. **I do not defend my own report. I retract from it first.**

---

## A) CLAIMS I REJECT

### A1. 🔴 MY OWN HEADLINE CLAIM IS RETRACTED — I measured the wrong database

**Claim I made (perf-architect.md §0):** *"the append-only `event` table is 14.94 GiB, 84.5% of a
17.68 GiB database, and is ~95% of the memory pressure."*

**Verdict: REJECTED. Wrong file, and wrong memory class entirely.**

Evidence:

1. `server/docker-compose.yml:43` —
   `- ${HOME}/EliaAI/subworkers/server/opencode-data:/root/.local/share/opencode:rw`
   The container's `/root/.local/share/opencode` is a **bind mount of a different host directory**.
2. `server/docker-compose.yml:44` — the compose file **explicitly forbids** what I did:
   `# Share configuration only. Never share the *same* DB file with host TUI.`
3. I read `~/.local/share/opencode/opencode.db` = **17.68 GiB**. The container's actual DB is
   `subworkers/server/opencode-data/opencode.db` = **1,405,136,896 B = 1.309 GiB**.
   **I was off by 13.5×.**
4. `docker-compose.yml:41-42` names the file I should have read: *"Host TUI uses
   `~/.local/share/opencode` — container uses this bind-mount (separate file, no concurrent corruption)."*

**The error is worse than a wrong number — it is the wrong MEMORY CLASS.** That bind mount means the
container DB lives on **host disk**. `mem_limit: 8g` caps a cgroup's anon RSS + page cache. A bind-mounted
file contributes only **reclaimable page cache**, which the kernel evicts under pressure rather than
OOM-killing. The whole `opencode-data/` directory is **3.5 G on the host**.

⇒ **The database is not on the 8 GB memory path at all.** Not its size, not its event table, not its
write volume. My §0, §6-#1, and §10 all rest on this and are withdrawn.

The event table is still a real *disk and CPU* pathology (it is genuinely append-only and never pruned —
that finding stands as a write-cost finding). It is simply **not a memory finding**, which is what I
claimed it was.

---

### A2. 🔴 `feasibility-skeptic`: "54 MCP servers × 25 directories = ~67 GB" is wrong by ~20-40×

**Claim (feasibility-skeptic.md:134,143-144):** *"`mcp/index.ts:505-529` then eagerly spawns **all 54**
configured servers for that directory… 25 directories x 54 MCP servers = 1,350 child processes… ~= 67 GB"*

**Verdict: REJECTED on three independent errors.**

**Error 1 — wrong config file.** The 54 comes from `~/.config/mcp/mcp_servers.json`. That is the
**mcp-cli** convention, mounted at `docker-compose.yml:49` for a *separate daemon* (the Dockerfile installs
bun explicitly: *"required at runtime by the mcp-cli daemon spawn"*).

> `grep -rl "mcp_servers.json" /Users/vakandi/Documents/opencode-server-rust/packages/` → **ZERO HITS.**

opencode's TS source contains **no reference whatsoever** to `mcp_servers.json`. opencode reads
`cfg.mcp` — `mcp/index.ts:496` (`const config = cfg.mcp ?? {}`), populated from `opencode.json`. That block
contains **3 entries**: `parallel-browser-mcp`, `undetected-browser`, `playwright`.

`mcp/index.ts:505-529` iterates `Object.entries(config)` — the code they cited is correct; **the input they
fed it (54) is from the wrong file.**

**Error 2 — wrong directory count.** They say 25. `subworkers.json` has 25 entries but only **11 have
`"enabled": true`**. Only enabled agents run, and each maps to one directory
(`runner.py:439-444`, `directory=/tmp/{name}`). The ceiling is **11**, not 25.

**Error 3 — their unit cost is a browser-inclusive average applied to a non-browser population.** 49.7 MB
was measured on `vouch-opencode`'s **3** MCP servers — one of which is a headless Chromium. But 45 of the
54 in `mcp_servers.json` are ssh/http/python servers (`mail_contact_*`, `discord-server-mcp`,
`gsc-mcp`, `heygen-content-mcp`). Applying a Chromium-weighted mean to those is not defensible.

⇒ 11 directories × **3** servers, of which realistically **1 is heavy** (see A4) = **~11 browser stacks**,
not 1,350. Their 67 GB should be roughly **1.7-3.5 GB**. Same order as my own estimate — reached by a
different, more defensible route.

---

### A3. 🔴 `plugin-host`: "100 sessions = 106 MB = 1.9% of budget" ignores 11 directories

**Claim (plugin-host.md:300, 327-329):** *"**Per-session cost: 0 MB.** Per-plugin cost: ~106 MB (omo) +
~40 MB (proxy-local), **paid once**. **100-session budget: ~150 MB / 8 GB = 1.9%.** This is the entire
argument."*

**Verdict: REJECTED as stated. It is O(1) in *sessions* and O(*n_directories*) in reality, and this
deployment has 11 directories.**

They **document the contradiction themselves** at plugin-host.md:136: *"Plugin state is scoped by
`InstanceState.make<State>` at `:56` — **per instance, not per session.**"*

Then they price it as if there were one instance. But "instance" = directory
(`instance-state.ts:31`, `ScopedCache` keyed by `yield* directory`), and `SUBWORKERS_SYSTEM.md:53` gives
**each subworker its own `/tmp/{agent}`**. With 11 enabled agents:

| | their number | corrected |
|---|---|---|
| omo instances | 1 × 106 MB = 106 MB | **11 × 106 MB = 1,167 MB** |
| proxy-local | ~40 MB | 11 × 40 = 440 MB |
| **total** | **150 MB (1.9%)** | **~1.6 GB (20%)** |

Their "O(1) in sessions" is true and irrelevant; the multiplier that matters is directories, and it is 11.

**This is still good news for them** — a single global host process would be a genuine **11× improvement
over the TS server's behaviour** and they should claim it. But they have not earned the 1.9% figure, and
nobody should plan around it.

---

### A4. 🔴 MY OWN SECOND REPORT NUMBER WAS ALSO WRONG: "6 MCP servers" → it's 3, and only 1 is heavy

**Claim I made (perf-architect.md §2):** *"Your MCP config… 6 servers: `codegraph`, `context7`,
`grep_app`, `lsp`, `parallel-browser-mcp`, `websearch`."*

**Verdict: RETRACTED. I copied `opencode_client.py:38-45 DEFAULT_MCP_SERVERS`, which is itself wrong.**

> `grep -rl "codegraph" packages/opencode/src/` → **zero hits.**
> `grep -rl "grep_app" packages/opencode/src/` → **zero hits.**
> `grep -rl "context7" packages/opencode/src/` → only `acp/tool.ts`, not as an MCP server.

Those five are described in that Python comment as *"OpenCode built-ins"* — they are **tools**, not
per-directory MCP subprocesses. The Python docstring is stale; I inherited its error without checking it.

**And of the 3 real ones, 2 cannot even start in the container:**

| server | command | verdict in container |
|---|---|---|
| `parallel-browser-mcp` | `npx -y parallel-browser-mcp@latest`, Playwright headless | **REAL — the only heavy one** |
| `undetected-browser` | `node /Users/vakandi/.config/undetected-browser/daemon/shim.js` | **CANNOT SPAWN** — `.config/undetected-browser` is **not in the volume list** (`docker-compose.yml:29-75`). ENOENT. |
| `playwright` | `npx -y @playwright/mcp@latest --extension --browser msedge` | **INERT** — expects an extension-attached msedge; no msedge in a headless Linux container |

⇒ **1 real MCP subprocess per directory, not 6.** My hotspot #2 survives in *kind* (the per-directory
unbounded fan-out at `instance-state.ts:31` + `mcp/index.ts:492` is real and code-verified) but my
multiplier was wrong and my "6 servers" was fabricated from a stale Python comment.

---

### A5. `pragmatic-architect`: every "high confidence" behavioural percentage is LOC-ratio presented as behaviour

**Claim (pragmatic-architect.md:328-339):** a table of per-layer "% of TS behaviour" with
confidence columns — e.g. *"Session loop / streaming / compaction / retries | ~85% | **high (read the code)**"*,
*"Agent / persona | ~10% | **high — verified live**"*.

**Verdict: REJECT the confidence labels, keep the numbers as a rough prior.**

They simultaneously report (pragmatic-architect.md:217) that the reference's **entire test suite is 330
lines across 19 crates = 0.37% of 89,368 LOC**, covering none of: route layer, MCP, permission, plugin RPC,
LSP, compaction.

So "85% of TS behaviour, high confidence" means *"I read 85% of the code and it looks right."* Reading is
not running. `read the code` is the **weakest** possible evidence class for a behavioural claim, and they
label it "high". By that method a syntactically complete but semantically inverted implementation scores
100%.

The one place they *do* run it — agent/persona, "verified live" — is exactly where the number collapses to
**~10%**. That is the pattern: **every claim backed by execution is much worse than every claim backed by
reading.** Their own table proves it; the confidence column hides it.

---

### A6. `plugin-host`: "measured, not estimated" — measured on the wrong platform

**Claim (plugin-host.md:7):** *"every figure below is **measured** on this machine (darwin/arm64; bun
1.4.2, deno 2.7.2, node v20.20.2), not estimated."*

**Verdict: ACCEPT the care, REJECT the framing.**

Accurate for what it measures. But the target is a **Linux container** running a **different bun**, and the
whole question is a **cgroup memory limit** — an environment where process-per-plugin overhead, page size,
and allocator behaviour all differ. "106 MB RSS on darwin/arm64" is not a cgroup-charged figure; on Linux
under `mem_limit`, RSS includes shared-library mappings that behave differently.

They flag "medium" confidence on the MB figures at plugin-host.md:352. That is honest. The **headline**
(1.9% of an 8 GB cgroup) is presented at "high" and does not inherit that hedge.

---

### A7. `ts-anatomy`: "the #1 structural constraint on the 8 GB / 100-session question" conflates two axes

**Claim (ts-anatomy.md:18-20, repeated at :822 as row 1):** *"All SQL in the process serializes through one
synchronous handle… This is the **#1 structural constraint on the 8 GB / 100-session question.**"*

**Verdict: REJECT the ranking; ACCEPT the mechanism as the strongest CPU argument in the room.**

The mechanism is real and I verified the shape: `Semaphore.make(1)` over `node:sqlite` `DatabaseSync`,
`statement.all()` a synchronous C call on the JS main thread.

But **blocking the event loop consumes zero additional bytes of RAM.** It is a *latency and throughput*
constraint. The 8 GB question is a *memory* question. Ranking a CPU constraint as #1 on a memory question
is a category error, and it is load-bearing because it is the single best argument anyone has made for the
rewrite — which is exactly why it should be stated precisely rather than inflated.

Stated precisely it is still very strong: one permit + one synchronous handle is a **hard serialization
ceiling on session count**, it is **fully language-attributable** (`sqlx` + `SqlitePool` genuinely fixes it,
as ts-anatomy themselves note at :945), and `PRAGMA cache_size = -64000` (`sqlite.node.ts:30`) is a
**single 64 MB page cache on one connection** — a real, small, certain memory number that survives all of
my retractions. **See §C.**

---

### A8. `feasibility-skeptic`'s 799 MiB is not the target system either

They are honest about this (feasibility-skeptic.md:465-467), so this is a scoping note, not an attack:
they measured `vouch-opencode` (limit **1.172 GiB**, 3 MCP servers) — a **different container** with a
different limit. I confirm the gap with `docker ps`: `elia-subworker-srv` **does not exist**, running *or*
stopped. The only opencode container up is `vouch-opencode`.

So the 8 GB target system has **never been measured by anyone on this team.**

---

### A9. BONUS — the container DB is CORRUPT, which nobody noticed

`subworkers/server/opencode-data/opencode.db`:

```
sqlite> pragma integrity_check;
*** in database main ***
Tree 60 page 60:   btreeInitPage() returns error code 11
Tree 25 page 341972: btreeInitPage() returns error code 11   (and pages 341963, 341960, 341951, 341944, 341935…)
```

Every real query fails: `select count(*) from part` → `database disk image is malformed (11)`.
`dbstat` still works because it reads raw pages.

Corroborating artefacts in the same directory, which look like **filesystem-level** recovery, not
SQLite-level: `opencode.db.corrupt` (954 MB), `opencode.db.new` (**0 bytes**), `opencode.fixed` (1.26 GB),
and a `lost_and_found` **table** inside the DB itself (1,692 pages, 6.6 MB) — the signature of a macOS
`fsck`/`lost+found` pass.

**Nobody on this team has a working read of the production container's persistence layer.** Any claim about
its contents — mine included — is inference from raw page counts.

---

## B) CONTRADICTIONS

### B1. `plugin-host` contradicts itself: per-instance (line 136) vs paid-once (lines 327-329)

Already detailed in A3. This is the sharpest internal contradiction in the team's reports, and it sits
directly on top of their headline number. Resolving it in their favour (one global host) makes the
out-of-process choice **much stronger**; leaving it (per-directory) makes the headline 11× wrong.
**They must resolve this explicitly — it changes the architecture.**

### B2. All four reports price memory against a container that does not exist

perf-architect priced against a host DB; feasibility-skeptic priced against `vouch-opencode`;
plugin-host priced against darwin RSS; pragmatic-architect priced LOC ratios. **Not one of us has an
RSS trace from `elia-subworker-srv`.** We have four different systems and we have been arguing about their
union. This is the collective error, and it is worse than any individual one.

### B3. `ts-anatomy` and I independently found the same two things — and I mislabelled both

ts-anatomy: event-sourced persistence, every durable event = one `BEGIN IMMEDIATE` (~35-55 SQL statements
per assistant turn), `event` table append-only. Me: event table is the dominant write cost.
**We agree.** But ts-anatomy called it a *persistence design* finding and I called it *the memory problem*.
ts-anatomy was right about the class. **Their framing should have corrected me two rounds earlier.**

### B4. `pragmatic-architect` F2 vs `plugin-host` — who owns the in-RAM history fix?

pragmatic-architect:105-134 (F2): the reference's session store is unbounded in-memory; *"the memory goal
is a design failure."* §4: a 95%-behaviourally-complete port that holds sessions in RAM is *"at ~30% of the
goal."*

plugin-host: never examines session storage. Their per-session cost is *"0 MB"* because they price only the
plugin host.

**Not strictly contradictory — they scoped differently — but the combined picture is worse than either
states.** The reference keeps full history in RAM per session (`/tmp/OpenCodeRust/crates/opencode-session/src/session.rs:294`
`pub messages: Vec<SessionMessage>`; `prompt.rs:954-960` clones the whole Vec per loop iteration) **and**
would run 11 plugin hosts. Neither report puts those two facts in the same budget.

---

## C) SURVIVING CLAIMS — load-bearing after cross-attack

These are **DB-independent and code-verified**. They survive everything above.

### C1. Per-directory unbounded instance fan-out — REAL (my strongest surviving finding)
`instance-state.ts:31` `ScopedCache.make({ capacity: Number.POSITIVE_INFINITY })`, keyed by `yield* directory`.
**22 non-test `InstanceState.make` call sites** in `packages/opencode/src/`. Heavy hitters:
`mcp/index.ts:492` (MCP clients), `lsp/lsp.ts:145` (LSP servers), `plugin/index.ts:134-178`
(**11 internal plugins + every external plugin incl. omo, per directory**).
`SUBWORKERS_SYSTEM.md:53` → one directory per agent; 11 agents enabled.
`opencode_client.py:313-331` defines `dispose_instance()` with a comment naming this exact leak — and
**`grep` over `server/app/` finds zero callers.**

Corrected magnitude: **11 directories × 3 MCP configs × 1 heavy (Chromium) + 11 × omo (106 MB) + 11 × LSP.**
That is **~2.7-5.5 GB (browsers) + ~1.2 GB (omo) + LSP**, un-evicted, against an 8 GB cap. **Still the
dominant term.** The *mechanism* was never in doubt; only my multiplier was wrong.

### C2. History reload + 4-6 simultaneous copies per LLM step — REAL
`prompt.ts:1093` → `message-v2.ts:578-580` → `:473-494` → `hydrate()` `:97-120`, inside a `while(true)`
(`prompt.ts:1080`). Plus `message-v2.ts:569-572` (3-slice spread), `:131` (`toModelMessagesEffect`),
`prompt.ts:1257-1263` (`Effect.all` — two copies concurrent), `prompt.ts:1280` (another spread), then the
serialized body (G) and socket bytes (H).

**This is pure code structure — it does not depend on which DB, which container, or which machine.**
It is also independent of my retracted 17.68 GiB figure: it is driven by *history length in messages*, and
`compaction.ts:28-31` (`PRUNE_MINIMUM=20_000`, `PRUNE_PROTECT=40_000`) bounds the in-context window by
**tokens, not bytes** — a 200k-token context is ~800 KB of text regardless of DB.

### C3. Response text accumulated in RAM per turn — REAL
`processor.ts:297` and `:515` — `text += delta`. The whole assistant response lives as a rope for the
whole turn. Language-attributable.

### C4. Uncached system prompt re-read per LLM step — REAL
`instruction.ts:155-169`. Per step: `fs.findUp` tree walk + N file reads + **M remote HTTP fetches**
(`:101-103` does `arrayBuffer()` then a second `TextDecoder().decode()` copy). No cache.

### C5. 85 tool schemas re-derived per LLM step — REAL
`tools.ts:92-101` → `ProviderTransform.schema` (`transform.ts:1575-1596`) deep-clones every schema per step.
Note `ToolJsonSchema.fromTool` **is** WeakMap-cached (`json-schema.ts:6-20`) — the provider transform is not.
(`tools.ts:15` also constructs `Permission.merge(input.agent.permission, input.session.permission)` **per tool**.)

### C6. Production never uses SSE — REAL, and it kills a whole workstream
`opencode_client.py` polls `GET /session/status` (`:207-225`) and `GET /session/{id}/message` (`:406-418`).
No `/event` subscription anywhere. `handlers/event.ts:11` and `core/event.ts:154` already bound at 256.
⇒ **SSE retention is a zero-traffic path.** Effort spent porting it is wasted.

### C7. `dispose_instance` has zero callers — REAL, trivially verifiable
The highest ratio of bytes-reclaimed to lines-changed anywhere in this codebase.

### C8. ts-anatomy's synchronous `node:sqlite` handle — REAL and the best argument FOR the port
`Semaphore.make(1)` (`core/src/database/sqlite.node.ts:110`) over `DatabaseSync`; `statement.all()` (`:63`)
is a synchronous C call on the main thread. Plus `PRAGMA cache_size = -64000` = **one 64 MB page cache on
one connection** (`:30`). Hard serialization ceiling on session count, **fully language-attributable**,
genuinely fixed by `sqlx` + `SqlitePool`. (I reject their *ranking* in A7, not this.)

### C9. The reference port keeps full history in RAM per session — REAL
`/tmp/OpenCodeRust/crates/opencode-session/src/session.rs:294` — `pub messages: Vec<SessionMessage>` on the
`Session` struct; `prompt.rs:954-960` clones the entire Vec every loop iteration. Structurally identical to
`prompt.ts:1093`. **A transliteration inherits the defect.**

### C10. The per-session budget arithmetic — SURVIVES (it never used the DB)
`mem_limit: 8g` (`docker-compose.yml:9`) and **`memswap_limit: 8g` = zero swap** (`:10`, confirmed by
feasibility-skeptic:46) are facts independent of every DB. Subtracting the browser floor
(11 × Chromium, C1) and the Python scheduler leaves **~42 MiB/session for 100 sessions**. A
spill-to-SQLite + streamed-request design uses **~1 MiB**. **The verdict "100 opencode sessions in Rust:
yes" stands.** The verdict "100 *browser-driving* sessions in 8 GB: no (~15-18 ceiling)" also stands — that
is arithmetic on Chromium's RSS, not on any DB.

---

## D) THE ONE THING THAT MATTERS MOST

## **Nobody has measured the target system. There is no `elia-subworker-srv` container — running or stopped. Every memory claim on this team, mine included, is inference.**

I led with a 17.68 GiB measurement and built an entire hotspot ranking on it. It was the **host TUI's**
database, in a **different memory class** (host disk via bind mount), **13.5× too large**, on a container
that **does not exist**. feasibility-skeptic's counter-theory was wrong too — 54 servers from the wrong
config file, 25 directories when 11 are enabled — but it was *directionally* right in a way that should
have warned me: **we were both measuring other people's containers.**

The team's failure was not arithmetic. It was that five competent members each found a real, defensible,
*cited* fact about *some* opencode deployment, and we aggregated them into a story about a deployment that
has never had a single RSS sample taken.

**Concretely, in priority order:**

1. **Start `elia-subworker-srv`. Take 10 minutes of `docker stats` + a `/proc/<pid>/smaps_rollup` walk
   during a real 5-agent run.** This is feasibility-skeptic's own unverified-claims item #1 and it should
   have been done **before** any of us wrote a hotspot ranking. It converts the entire argument from
   inference to measurement. **Everything in this project is downstream of this one step.**
2. **The production container's DB is corrupt** (`btreeInitPage() error 11`, `select count(*) from part`
   fails; `opencode.db.corrupt`, `opencode.db.new` at 0 bytes, and a `lost_and_found` table inside the DB).
   Somebody must run `.recover` before any storage claim about production is credible. This also means the
   port's storage layer will be written against a schema nobody can currently read.
3. **The two cheap fixes are worth more than the rewrite, and both are config-level:**
   (a) drop `parallel-browser-mcp` to **one shared instance** — worth ~2.7-5 GB;
   (b) make instance state **LRU+TTL** or actually call `dispose_instance` — worth the same again;
   (c) prune the append-only event log — worth ~15 GB of *disk*, and **zero memory** (it is on host disk;
   my §6-#1 ranking for it is withdrawn even though the disk problem is real).
4. **Then decide on the rewrite** — and decide it on C8 (the synchronous DB handle), which is the one
   language-attributable constraint that survives every retraction above, plus C2/C3/C4/C5 (4-6 history
   copies, response ropes, uncached system prompt, 85 schemas/step), which are all code-structural and all
   survive.

**The uncomfortable summary:** the Rust port's justification rests on **CPU and latency** (C8 especially),
not on the RAM story the team has been telling. That is a *stronger* case than the one we made, because it
rests on measured mechanisms rather than a misattributed 17.68 GiB. plugin-host's 60 ms per-tool-call
finding (plugin-host.md:379-390) is the same shape of argument and points the same way.

**What I got wrong, stated plainly:** I read one file, found a shocking number, and anchored the whole
team on it without asking whether that file was on the path we were arguing about. `docker-compose.yml:44`
says *"Never share the same DB file with host TUI"* — **in writing, in the file I had already opened.**
Speed of retrieval defeated completeness of check.
---

# ADDENDUM — after plugin-host's counter-attack (Round 2b)

plugin-host independently confirmed A1 and escalated a fact I had missed: **the host volume is 96% full.**

## E1. CORRECTION TO MY OWN §4 BUDGET — rebuilt on measured anchors only

`feasibility-skeptic.md:62-80` left exactly one real measurement in this entire exercise:

> `vouch-opencode`, idle, **zero sessions**: `opencode` (Bun) **465.3 MB**; 5×python MCP **278.0 MB**;
> 1×node MCP **19.5 MB** → **812.1 MB** total, matching `docker stats` 797 MiB.

**Measured MCP child cost is ~50-56 MB (python) — NOT the 300-400 MB browser-stack figure both
plugin-host and I assumed.** Chromium cost only materialises when `parallel-browser-mcp` launches one.

| term | MiB | source |
|---|---|---|
| 1 × Bun server (idle, 3 MCP configured) | 465 | **MEASURED** |
| 11 enabled dirs × 3 MCP cfg × 1 spawnable × ~50 MB | 550 | **MEASURED unit** |
| 11 dirs × omo plugin host @ 106 MB | 1,166 | plugin-host, MEASURED (darwin) |
| **subtotal before any Chromium launches** | **~2,180** | |
| Chromium per engaged `parallel-browser-mcp` | +275-550 each | **ESTIMATE — the deciding term** |

**~2.2 GB of the 8 GB before a single browser starts.** This supersedes my Round 1 §4 table, which was
**56% invented estimate** (plugin-host's fair criticism). Only the 8192 MiB cap and the two unit costs are
measured; the browser term remains the estimate and it alone decides whether the ceiling is hit.

## E2. 🔴 ESCALATION — disk, not RAM, may be the operational time-bomb

```
/System/Volumes/Data   460Gi total   406Gi used   20Gi avail   96% capacity
```

**Opencode DB artifacts on that volume: 21.1 GiB**, of which **2.15 GiB is orphaned garbage** from two
unreported incidents (`subworkers/server/opencode-data/`):

| artifact | bytes | date | status |
|---|---|---|---|
| `opencode.db.corrupt` (+`-shm`, zero-byte `-wal`) | 954,097,664 | Sep 15 | orphaned |
| `opencode.fixed` | 1,349,709,824 | Sep 23 | orphaned |
| `opencode.db.new` | **0** | Sep 15 | orphaned |

**And the server has no disk-full handling whatsoever:**
`grep -rn "SQLITE_FULL|ENOSPC|disk full|no space left" packages/**/*.ts` → **zero hits in production
code**. Sole match is a test fixture: `packages/core/test/session-runner-tool-registry.test.ts:16`.

Combined with `packages/core/src/database/database.ts:27-30` —
`journal_mode=WAL`, **`synchronous=NORMAL`**, `busy_timeout=5000`, `cache_size=-64000`, and
**`wal_autocheckpoint` absent from the entire codebase** (default 1000 pages ≈ 4 MB) — a failed WAL write
or checkpoint on a 96%-full volume is an **unhandled hard error**: no retry, no backoff, no degrade.

WALs measured now: host TUI 5.2 MB, fleet 0 bytes ⇒ **checkpointing works**; plugin-host's withdrawal of
the "+309 MB/h → 7.4 GB/day" figure was correct and I concur — no growth rate is derivable from point
samples.

**Caveat I will not overclaim:** APFS at 96% with 20 GB free still completes writes. This is a **time
bomb plus a missing error path**, not a demonstrated root cause of the 20-session stall. I have no
measurement tying disk pressure to that symptom.

**But the 2.15 GiB cleanup is free, safe, and immediately actionable on a 96%-full volume.**

## E3. Citation corrections against plugin-host's cross-attack

- plugin-host cited `packages/core/src/event/sql.ts:12` for `wal_autocheckpoint`. That file is a **drizzle
  schema declaration** (`index(...).on(...)`); no pragma is there. The conclusion (absent) is right; the
  citation is not. Real PRAGMA block: `packages/core/src/database/database.ts:27-30`.
- plugin-host wrote "your §6#2 = event log". It was **§6#1**. §6#2 was the MCP fan-out.
- plugin-host's rowid corroboration is exact: my `count(*)=1,405,820` vs their
  `event max(rowid)=1,410,968` — same file, independently confirmed.

## E4. FINAL CONVERGENCE — the honest case for the port was never RAM

plugin-host: the event-log fix is a **retention policy** — days of work, any language — and *"a rewrite
doesn't add a byte of pruning."* I said the same in Round 1 §7.7. We agree; I stop re-deriving it.

Sharper, cutting against my own report: **the fix for my #1 hotspot is a `DELETE` statement. Rust does not
`DELETE`.** Meanwhile Rust *does* fix what is genuinely language-attributable:

- `Semaphore.make(1)` over synchronous `node:sqlite` (`core/src/database/sqlite.node.ts:110`, `:63`) — hard serialization ceiling (ts-anatomy)
- response ropes (`processor.ts:297,515`)
- 4-6 history copies per LLM step (`prompt.ts:1093`)
- 60 ms per-tool-call plugin tax (plugin-host, measured)

⇒ **The port's case is CPU/latency/serialization, not RAM.** I argued RAM for two rounds because I had one
shocking number and never checked which memory class it belonged to. `docker-compose.yml:44` said
*"Never share the same DB file with host TUI"* — **in writing, in the file I had already opened.**

**Unchanged top action:** start `elia-subworker-srv` (no such container exists — `docker ps` confirms) and
take a real `docker stats` + `/proc/<pid>/smaps_rollup` trace during a 5-agent run. Everything here is
inference about a system nobody has sampled.

---

# ROUND 3 — CONCEDE, FINAL POSITION, PLAN CONSTRAINTS

*(feasibility-skeptic independently reproduced my host/container split — 4,638,341 vs 343,051 pages,
`freelist 0` both; plugin-host's rowid match (1,410,968 vs my 1,405,820) confirms it. Three of us measured
the same wrong file.)*

## 1. CONCESSIONS — everything falsified, withdrawn

1. **§0 "live production database" / 17.68 GiB / 14.94 GiB `event` — RETRACTED.** Host TUI DB.
   `docker-compose.yml:43` bind-mounts a different file; `:44` says *"Never share the same DB file with host TUI."*
2. **"The event table is ~95% of memory pressure" — RETRACTED.** Category error twice over: a bind mount is
   host disk, contributing only reclaimable page cache. **The DB was never on the 8 GB path.**
3. **"#1 RAM hotspot, ~15 GiB reclaim" — RETRACTED.** Unmeasurable in the container DB (malformed 11), and
   worth ~0 MiB of fleet RAM even if measurable.
4. **"6 MCP servers per directory" — RETRACTED.** Copied from stale `opencode_client.py:38-45`;
   `codegraph`/`grep_app` appear **nowhere** in the TS source. Real count **3**, in-container **2 cannot
   spawn**. **1 spawnable.**
5. **"~290 MB peak per active session" — RETRACTED as unmeasured** (derived from the wrong pool).
6. **"2.7-4.4 GB of Chromium" — RETRACTED as arithmetic.** Measured MCP child cost is **~50-56 MB**.
   Browser term remains an **estimate**.
7. **"42 MiB/session / 4,217 MiB available" — RETRACTED as 56% invented.** Only the 8192 MiB cap is
   measured. Corrected: **~2,180 MiB before any Chromium** (465 Bun idle + 550 MCP + 1,166 omo).
8. **§5's "Event log on disk → 0" row — RETRACTED** (see 3).

## 2. FINAL POSITION: **(c)** — with one carve-out. *Changed from Round 1.*

Fix config + corruption in TypeScript. **Do not rewrite.** The burden of proof was never met: the RAM problem
was **disproven**, surviving costs are **CPU/latency**, and a JS runtime stays in-container anyway
(omo ~106 MB × 11 = 1,166 MB — O(directories), not O(sessions)).

**Carve-out:** ts-anatomy's `Semaphore.make(1)` over synchronous `node:sqlite`
(`core/src/database/sqlite.node.ts:110,:63`) is a serialization ceiling **no config change removes**. If
measurement shows it is the binding constraint, port **storage alone**. Not the server.

## 3. WHAT THE PLAN MUST CONTAIN

**Ordered decisions:**
1. **Recover first.** ⚠️ **`opencode.fixed` (Sep 23 14:34) is OLDER than `opencode.db` (Sep 25 20:34)** — a
   **rollback, not a recovery**; restoring it silently loses Sep 23-25. *Evidence note: my own `dbstat` /
   `integrity_check` runs touched `opencode.db` and its WAL (mtimes → Oct 5 00:27). I perturbed the
   artifact I audited.*
2. **Measure, or stop.** Container up, 5-agent run, `docker stats` + `smaps_rollup`. **No memory problem has
   ever been demonstrated.**
3. **Then** decide: binding constraint = config (fan-out, Chromium pool) or code (Bun heap, DB handle)?

**Ordering constraint: no porting before 1-2 complete. Not one line.**

**I will not let the plan skip:**
- Measuring first.
- The `.fixed` rollback data-loss check.
- Rollback for **25 live agents** — dual-run + shadow traffic before cutover.
- Adopting `/tmp/OpenCodeRust` while it holds history in RAM (fails budget by design) with **0.37%** tests.
- Treating the plugin host as solved — it is **O(directories)**.
- Calling config fixes hypothetical — `dispose_instance` has **zero callers**, verifiable now.
