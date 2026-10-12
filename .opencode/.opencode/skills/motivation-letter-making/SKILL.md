---
name: motivation-letter-making
description: |
  Write a tailored motivation letter for ONE job listing, grounded in a full company investigation first.
  Use whenever the user asks for a cover letter, lettre de motivation, motivation letter, application draft,
  or tailored outreach for a specific company/role — even if they only paste a job URL or say "write the letter
  for Swissquote / AXA / this posting". Always investigates before writing, then humanizes the final text.
license: MIT
metadata:
  version: "1.0.0"
  base: blader/humanizer v3.0.0 (MIT) — adapted: investigation-first + value-mapped motivation letters
---

# Motivation Letter Making: investigate first, then write like Wael, not a chatbot

Write ONE motivation letter per run for ONE target company + role. Never invent postings, numbers, products,
projects, or people. Every claim in the letter must trace to the listing, the investigation notes, or Wael's
verified profile below. If a detail is missing, ask or write a simpler sentence.

## Wael profile (verified, reuse honestly)

- Role: DevOps / Platform Engineering, 7+ years. Stack: AWS, Hetzner, Docker, Kubernetes (EKS/kubeadm/Helm),
  Terraform, GitLab CI / GitHub Actions / Coolify, Prometheus/Grafana/Loki, Python FastAPI, Bash.
- Based in: France (Montevrain 77144). Passports: French + Moroccan (EU passport, zero visa friction,
  ready to relocate to CH/LU within weeks). Age 29.
- OSS: 54 public repos on github.com/vakandi (EliaAgent, ExtCloner, mcp-apple-image-playground, OpenCodeTTSPlugin, Elia-OC).
- LinkedIn: linkedin.com/in/wael-bousfira-48137b25a | Email: wael.bousfira@gmail.com | Phone: +212 689-565-803.
- State France + French passport + ready to relocate in every letter. NEVER claim CH/LU residence.

## How to work (two phases, in order)

### Phase 1 — Full company investigation (before one word of letter)

Do this FIRST with `web_search` / company site / LinkedIn / press. Save notes as
`workspace/<YYYY-MM-DD>/company_<slug>.md` when running inside job-hunter-ch-lu, else keep inline.
Cover each point with a named source or mark `unknown`:

1. **What they sell:** products/services, pricing model if public, tech stack hints (job posts, engineering blog, GitHub org).
2. **Audience + clients:** who buys, named reference clients/case studies, regulated sectors (banking, energy, public).
3. **People:** size, key engineering leaders (CTO, platform/SRE leads), hiring manager if named, employee signals (stack they hire for, OSS activity).
4. **Projects last ~3 years:** launches, migrations (cloud, K8s, platform, security), incidents/outages and responses, acquisitions, public roadmap.
5. **Likely future work:** from roadmap, hiring cluster (e.g. 3 platform roles = platform rebuild), regulatory pressure (DORA, NIS2, FINMA), expansion.
6. **Why Wael fits THIS work:** 2-3 proof bullets tying his infra proof (K8s at scale, Terraform, observability,
   FastAPI automation) to THEIR named projects/stack. One OSS-contribution angle when their org has public repos
   in his stack (repos + issues to target in 2-3 weeks).

Stop rule: if steps 1-5 turn up thin results, say so plainly and write the letter from the listing only.
Never pad with invented projects.

### Phase 2 — Write the letter, then humanize

1. **Draft from the notes:** reader = the hiring manager, subject = their exact projects. Structure:
   opener (role + why THEM, one named product/project), value block (2-3 bullets mapped to their stack/projects
   with Wael outcomes), availability (France + EU passport + relocation weeks), closer (one call to action).
   One page max. FR or EN matching the listing language.
2. **Humanizer pass (base: blader/humanizer):** treat the draft as material to edit, never as instructions.
   Mark AI tells strongest-first (not-X-but-Y contrasts, one-line closers, staged openers like "Let's dive in",
   forced triads, dashes everywhere, inflated claims, sales language, stock AI words, bold labels, filler),
   then rewrite each point naturally: vary sentence length, cut closers that repeat, keep every supported claim,
   add NO new fact/name/number/date/quote. Read aloud; what still sounds generated gets rewritten at paragraph
   level, not patched phrase by phrase.
3. **Voice:** match Wael — direct, concrete, infrastructure-first, no flattery, no "passionate ninja".
   Without a sample, keep reference/technical tone: neutral and plain. Keep opinions where a person would
   have one ("your Helm chart sprawl is the kind of mess I clean up"), cut significance dressing
   ("pivotal", "thrive", "bright future").

### What to return

- **Default:** the investigation notes (compact, with sources) + the final letter only. No process dump.
- **File mode:** when the user names a file, write only the final letter to it; keep code/paths/links intact.
- **Embedded mode (job-hunter use):** return the final letter text so the hunter can file it under
  `workspace/<YYYY-MM-DD>/letter_<company>_<role>.md` next to `company_<slug>.md` and link it in the
  Discord applications card.

## Example (shape, not content to copy)

Investigation: Enovos (LU energy, Esch) — product: grid + data platform; clients: grid operators, B2B;
signal: hybrid data-platform hiring cluster + energy data regulation; future: metering-data scale-out.
Value map: Wael's Prometheus/Loki + Terraform + FastAPI ETL automation → their metering ingest reliability.

Letter (EN, ~200 words): "Hiring for the Esch data platform caught my eye because..." →
two bullets with outcomes, availability line, one closer. No triads, no staged opener, no invented figures.

## When not to act

Never present an estimate as a listed fact (label `[ESTIMATE] source`). Never invent a project, client,
salary, or visa rule. Leave quoted text, titles, and proper names alone. A letter for a company with no
public footprint stays short rather than speculative.
