---
name: job-hunter-agent-creator
description: Builds an omp-server job-hunter subworker with the right personality from a person's profile (skills, experience, location, target countries, salary floor, priorities). Intake → persona → deploy via omp-subworker-creator → polish via prompt-enhancer. The skill NEVER hunts jobs itself — it only builds the subworker whose PROMPT.md does the hunting. Trigger on "build me a job hunter", "create a job-hunter subworker", "job hunting agent", or "adapt this to my needs and country".
---

# Job Hunter Subworker Builder

**Sole purpose:** build an omp-server subworker with the right personality (PROMPT.md) for personalized job hunting. That is ALL this skill does.

**Hard rule — NEVER hunt yourself.** This skill MUST NOT call Indeed MCP, LinkedIn MCP, agent-browser, `web_search`, or any job-board tool. Those tools belong inside the subworker's PROMPT.md as ITS instructions, not yours. Your only tools: `ask` (intake), file writes (PROMPT.md), `subworkers.json` registration, `/config/reload`, and `task({agent: "prompt-enhancer"})` for polish.

## Phase 1 — Build the profile (intake)

Never invent details. If the user already gave you a resume, LinkedIn, or a description of themselves earlier in the conversation, extract what you can from it and only ask about what's missing. Otherwise ask concisely (use `ask` where the answers are pick-able, plain text where they're not) for:

1. **Core profile**: role/field, years of experience, key skills, education, current location, nationality/passport(s) (this drives visa feasibility).
2. **Where**: open to relocation? Which countries/cities are acceptable, preferred, or off-limits? Remote-only, hybrid, or on-site okay?
3. **Money**: minimum acceptable salary, target salary, currency, and whether that's gross/net. Raw salary or cost-of-living-adjusted comparison?
4. **Advantages they care about** (rank or multi-select): visa sponsorship, remote flexibility, health insurance quality, PTO/parental leave, equity/bonus, job security, career growth/title, work-life balance, tax burden, relocation package.
5. **Deal-breakers**: anything that disqualifies a role outright.
6. **Timeline**: actively applying now, or exploratory research.

Read `references/intake_questions.md` for the full question bank and good defaults if the user wants to move fast ("just use sensible defaults").

## Phase 2 — Generate the persona ( = the subworker's personality)

Generate a persona document using the template in `references/persona_template.md`. Fill every placeholder — do not leave brackets. This persona IS the subworker's personality: it will be embedded verbatim as the core of the subworker's PROMPT.md in Phase 3.

The persona must encode, in the agent's own voice:
- Its sole mandate: find and rank the best real job opportunities for this specific person — nothing else.
- Its ranking formula, in priority order, derived from the user's answers (hard filters first, then weighted priorities — adapt weights to what the user actually said).
- Its research method: real listings only, never fabricated. Salary estimates labeled as estimates. Cross-country comparisons adjusted per the agreed basis.
- Its output format: a ranked shortlist (5–8), each with role, company, location, verified/estimated pay, rank reasons, and a one-line fit note.
- Its tone: direct, unsentimental about trade-offs.

Show the persona to the user for approval before deploying. Do NOT start Phase 3 until they confirm (or pre-approved it in the same request).

## Phase 3 — Deploy via omp-subworker-creator

1. **Load `skill://omp-subworker-creator`** — read it FIRST. It defines the exact pipeline (workspace dir → PROMPT.md → `subworkers.json` → hot-reload → verify on `:5677`). Follow its Steps 0–5 exactly.
2. The subworker's PROMPT.md = OMP HOME header + Phase 2 persona (verbatim core) + a HUNT TOOLING section specifying ITS search tools:
   - hasdata Indeed MCP (`xd://mcp__hasdata_indeed_listing_getjoblistings`) with the correct country `domain` per target market (e.g. `ch.indeed.com` + `location: Switzerland`, `lu.indeed.com` + `location: Luxembourg`) — always specify `domain`, omitting it returns US-only results
   - LinkedIn MCP (`xd://mcp__mcp_server_linkedin_search_jobs`) by keyword + location, cross-referenced against Indeed
   - agent-browser fallback for country job boards (jobs.ch, jobup.ch, workland.lu, jobs.lu, Welcome to the Jungle)
   - `web_search` for company career pages, Glassdoor/Levels.fyi comparables, government salary data
3. `model` MUST be a live `zen-free/*` id (never `opencode-zen/*`). `enabled: false` until the verify run passes.
4. Verify: `POST /config/reload` → `GET /status/<name>` → `POST /trigger/<name>` → frames stream text+reasoning+tool.

## Phase 4 — Finish with prompt-enhancer

1. **Call `task({agent: "prompt-enhancer", task: "..."})`** with: the PROMPT.md path, the Phase 2 persona, and the hunt-tooling spec — ask for deeper research flow, tighter wording, no filler, correct tool references.
2. After it finishes, re-verify the subworker still passes `GET /status/<name>` + `POST /trigger/<name>`.
3. Report completion with the verification checklist from `omp-subworker-creator`.
