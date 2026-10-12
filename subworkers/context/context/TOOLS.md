# TOOLS.md - Quick Reference

> ⚡ **codemem** (AI Memory) — Auto-captures session context. USE IT to:
> - Remember tasks, decisions, blockers across sessions
> - `codemem search "query"` — поиск memories
> - Auto-injects relevant memories before each prompt
> ⚠️ NO manual commands — works automatically via plugin

> ⚠️ All commands here = **terminal commands** — use the `bash` tool (not skills).

---

**All MCP tools are ALWAYS 100% available.** If any MCP tool fails:

1. **WhatsApp issues** → Run:
   ```bash
   /Users/vakandi/Documents/mcps_server/restart-whatsapp-bridge.sh restart
   ```

2. **Playwright issues** → Run:
   ```bash
   /Users/vakandi/Documents/mcps_server/restart_clean_mcp_playwright.sh
   ```

3. **Any other MCP failure** → **IMMEDIATELY** run a new ulw-loop to investigate:
   ```
   /ulw-loop
   ```
   See section **"ULW-Loop (Autonomous Task Execution)"** (lines 174-200) for full details on how to run and use it.

   Then send a report to Discord #reports with:
   - What MCP failed
   - Error messages
   - Investigation steps taken

**MCP Server Location**: `/Users/vakandi/Documents/mcps_server/`

## ⚠️ Jira MCP - Known Issues

**Problème**: Certains tools ne sont pas exposés (create_issue, jira_search_issues)
**Solution**: Utiliser `jira_get_project_issues` pour lister, puis navigateur web pour créer les tickets
**Note**: Le MCP peut lire mais pas créer certains types de tickets

## 📱 Telegram Username
- **token_detective (Wael)**: @490522699404148756

---

## 🔊 Voice Message Handling (WhatsApp Only)

**CRITICAL**: When receiving or sending voice messages via WhatsApp, ALWAYS check for existing transcripts before transcribing.

### Voice Message Workflow

1. **Detect voice message** → Check `hasMedia` and `mediaType` in message
2. **Extract date** → Get timestamp from message → format as `YYYY-MM-DD`
3. **Check docs folder** → Look for `/Users/vakandi/EliaAI/docs/{YYYY-MM-DD}/`
4. **Check existing transcript** → Search for `{wa|tg}_{msg_id}` in filenames
5. **If already transcribed** → Skip processing
6. **If not transcribed** → Download audio → Transcribe with Whisper → Analyze as text

### Transcript Filename Convention

```
run_{source}_{msg_id}_{DD_mmm_YYYY_HHMM}.md
```

| Field | Description | Example |
|-------|-------------|---------|
| `source` | `wa` (WhatsApp) | `wa` |
| `msg_id` | Message ID from the platform | `msg_abc123` |
| `DD_mmm_YYYY_HHMM` | Day, month (French), year, time | `29_mars_2026_0945` |

**Examples:**
- WhatsApp: `run_wa_msg_abc123_29_mars_2026_0945.md`

### Transcript Check Logic

```bash
# Check if message already transcribed
MSG_ID="<message_id>"
TODAY=$(date +%Y-%m-%d)
DOCS_DIR="/Users/vakandi/EliaAI/docs/$TODAY"

# Direct filename check (fastest)
ls "$DOCS_DIR"/*_"$MSG_ID"_*.md 2>/dev/null && echo "Already transcribed" || echo "Need transcription"

# Or grep for msg_id in any file
grep -rl "$MSG_ID" "$DOCS_DIR"/*.md 2>/dev/null && echo "Already transcribed"
```

### Voice Message Transcription Command

```bash
# Download audio first via MCP, then transcribe
whisper /path/to/audio.ogg --model large-v3 --language French --task transcribe
```

### Transcript File Content Template

```markdown
# Voice Message Transcript

**Source:** WhatsApp
**Message ID:** {msg_id}
**Timestamp:** {DD_mmm_YYYY_HHMM}
**Sender:** {sender_name/number}

---

## Transcription

{transcribed_text_here}

---

## Context & Analysis

{analysis_of_content}
```

---

## 🖼️ Image Handling (WhatsApp)

```bash
# Detect: hasMedia=true, mediaType="image" → Download → Analyze
mcp-cli call whatsapp download_media '{"message_id":"<id>","chat_jid":"<jid>","save_path":"/Users/vakandi/EliaAI/docs/YYYY-MM-DD/"}'
# Single image
mcp-cli call vision-mcp analyze_image '{"image_paths": ["/path/to/image.jpg"], "prompt": "Describe this image", "model": "nvidia/nemotron-nano-12b-v2-vl:free"}'
# Multi-image (supported by Qwen VL, Gemma 3, Mistral, Llama vision)
mcp-cli call vision-mcp analyze_image '{"image_paths": ["img1.jpg","img2.jpg"], "prompt": "Compare these images"}'
# Save as: img_wa_{msg_id}_{DD_mmm_YYYY_HHMM}.md
```

---

## MCP-CLI (All External Services)

**⚠️ IMPORTANT:** `mcp-cli` is a **shell command** (terminal/bash), NOT a tool within OpenCode. It wraps all MCP servers and exposes them via command-line interface.

**Info:** `mcp-cli -h` or `mcp-cli --help`

```bash
# List servers (shell command)
mcp-cli

# All commands below are SHELL COMMANDS - run them in terminal or via bash

# WhatsApp
mcp-cli call whatsapp list_chats '{"limit":20}'
mcp-cli call whatsapp list_messages '{"chat_jid":"...","limit":30}'
mcp-cli call whatsapp send_message '{"recipient":"...","message":"..."}'
mcp-cli call whatsapp download_media '{"message_id":"...","chat_jid":"..."}'

# 📅 MESSAGE SCHEDULING (NEW!)
# Schedule a message for future delivery (ISO-8601 format)
mcp-cli call whatsapp schedule_message '{"recipient":"165558221861055","message":"Bonjour!","scheduled_time":"2026-04-22T09:00:00"}'

# List all scheduled messages (optional: filter by status: pending/sent/failed/cancelled)
mcp-cli call whatsapp list_scheduled_messages '{"status":"pending","limit":10}'

# Cancel a pending scheduled message
mcp-cli call whatsapp cancel_scheduled_message '{"message_id":123}'

# Delete a scheduled message from database
mcp-cli call whatsapp delete_scheduled_message '{"message_id":123}'

# Recipient formats:
# - Phone number with country code, no + (e.g., "165558221861055")
# - JID (e.g., "123456789@s.whatsapp.net")
# - Group JID (e.g., "123456789@g.us")

# ⚠️ VOICE MESSAGE HANDLING: When listing messages, check hasMedia + mediaType="ptt"
# If voice message detected:
# 1. Check date → /Users/vakandi/EliaAI/docs/{YYYY-MM-DD}/
# 2. Search for "wa_{msg_id}" in filenames to check if transcribed
# 3. If not → download_media → whisper → save as run_wa_{msg_id}_{DD_mmm_YYYY_HHMM}.md

## 🎨 Apple Image Playground (apple-image-playground)

**Server:** `apple-image-playground` | **Tools:** 17 | **Status:** ✅ Active

```bash
# List engines (apple, pollinations)
mcp-cli call apple-image-playground list_engines '{}'

# List presets (40+ platform presets for Instagram, TikTok, LinkedIn, etc.)
mcp-cli call apple-image-playground list_presets '{}'

# List styles (animation, emoji, illustration, sketch, messages-background)
mcp-cli call apple-image-playground list_styles '{}'

# Generate with Apple Intelligence (on-device, stylized)
mcp-cli call apple-image-playground generate_image '{"prompt":"a futuristic dashboard","style":"illustration","engine":"apple"}'

# Generate photorealistic (Pollinations.ai, free, cloud)
mcp-cli call apple-image-playground generate_image '{"prompt":"a modern office","engine":"pollinations"}'

# Add text overlay
mcp-cli call apple-image-playground add_text_overlay '{"input_path":"/path/to/image.png","text":"MirorPay","font_size":72,"position":"center"}'

# Smart crop with face detection
mcp-cli call apple-image-playground smart_crop '{"input_path":"/path/to/image.png","target_width":1200,"target_height":630}'
```

**When to use Apple Image Playground vs HeyGen:**
- **Apple Image Playground** = better quality for human-facing content (community posts, social media, Instagram stories)
- **HeyGen HTML/CSS** = better for blog posts (SEO — text must be readable by search engines, controlled by HTML/CSS)
- Apple Intelligence styles: `animation`, `emoji`, `illustration`, `sketch`, `messages-background`
- Output: 1536×1536 PNG, ~3-4MB, ~10-15s generation

## 📡 Discord MCP — discord-server-mcp

**Quick Reference:**
```bash
# Discover all operations available
mcp-cli call discord-server-mcp discord_discover '{}'

# Get schema for a specific operation
mcp-cli call discord-server-mcp discord_get_schema '{"operation":"messages.list"}'
```

### Available Tools

| Tool | Purpose | Returns |
|------|---------|---------|
| `discord_discover` | List all operations | Operation names |
| `discord_get_schema` | Get params for operation | JSON schema |
| `discord_execute` | Run any Discord operation | Operation result |
| `discord_send_message` | Send text to channel | Message object |
| `discord_send_file` | Upload file to channel | Message object |
| `discord_get_all_messages` | Bulk read channel | Message array |

### Operation Syntax (discord_execute)

```bash
# List all channels in guild
mcp-cli call discord-server-mcp discord_execute '{"operation":"channels.list","params":{}}'

# List messages from channel (last N)
mcp-cli call discord-server-mcp discord_execute '{"operation":"messages.list","params":{"channel_id":"CHANNEL_ID","limit":20}}'

# Messages from last N hours
mcp-cli call discord-server-mcp discord_execute '{"operation":"messages.list_range","params":{"channel_id":"CHANNEL_ID","hours":12,"limit":50}}'

# Messages after timestamp (ISO 8601)
mcp-cli call discord-server-mcp discord_execute '{"operation":"messages.list","params":{"channel_id":"CHANNEL_ID","after_timestamp":"2026-04-04T12:00:00Z","limit":50}}'

# Server/guild info
mcp-cli call discord-server-mcp discord_execute '{"operation":"guild.get","params":{}}'
```

### Send Messages

```bash
# Send text (handles special chars properly)
mcp-cli call discord-server-mcp discord_send_message '{"channel_id":"CHANNEL_ID","content":"Message here 🚀"}'

# Send file with optional caption
mcp-cli call discord-server-mcp discord_send_file '{"channel_id":"CHANNEL_ID","file_path":"/path/to/file.md","content":"📋 Report attached"}'

# Fallback: upload to tmpfiles.org if direct fails
curl -X POST https://tmpfiles.org/api/v1/upload -F "file=@/path/to/file.pdf"
```

### ⚠️ Troubleshooting

| Error | Cause | Fix |
|-------|-------|-----|
| 401 Unauthorized | Bot token expired | Get new token from Discord Developer Portal |
| Unknown Operation | Typo in operation name | Run `discord_discover` to list valid operations |
| Missing Params | Required field missing | Run `discord_get_schema '{"operation":"XXX"}'` |
| Channel Not Found | Wrong channel_id | Check channel mapping below |

---

## 🏢 EliaWorkSpace — Channel Mapping

**Guild ID:** `1489242790444662885`

### MIRORPAY (Payment Platform)

**Category ID:** `1506669201715367938`
| Channel | Channel ID |
|---------|------------|
| 📊-reports | `1489244810777727046` |

### ELIA-HQ (System - Elia AI)
| Channel | Channel ID |
|---------|------------|
| 💡-urgent | `1489244806310793216` |
| 📊-reports | `1489244810777727046` |
| 📝-activity-logs | `1489244812107317402` |
| 📚-knowledge | `1489244815790182450` |
| ✅-tasks-tracker | `1489244818134794330` |
| 💚-health-checks | `1489247935807099020` |

### BEN2LUXE (Jira: BEN) - DIFFERENT BUSINESS
| Channel | Channel ID |
|---------|------------|
| 🛍️-products | `1489244857250615416` |
| 📦-orders | `1489244862871244950` |
| 👥-clients | `1489244868235755580` |
| 📱-social-media | `1489244873847734292` |
| 📣-marketing | `1489244878431846523` |
| 🗂️-management | `1489246873998065745` |
| tiktok-ai-to-copy | `1489247163824345228` |

### COBOU-AGENCY (Jira: COBOUAGENC) - DIFFERENT BUSINESS
| Channel | Channel ID |
|---------|------------|
| 🚀-projects | `1489244906013593642` |
| 👥-clients | `1489244911449538680` |
| 💻-dev-work | `1489244916352684045` |
| 💰-invoices | `1489244921180455035` |
| management | `1502898800690204683` |

### ZOVABOOST (Jira: ZOVAPANEL) - DIFFERENT BUSINESS
| Channel | Channel ID |
|---------|------------|
| 🖥️-panel | `1489244946673176618` |
| 👥-clients | `1489244951861661787` |
| 🎧-support | `1489244963261780173` |

### MAYAVANTA (Jira: MAYA) - DIFFERENT BUSINESS
| Channel | Channel ID |
|---------|------------|
| 🛎️-concierge | `1489244961269485711` |
| 🚗-car-rental | `1489244967095238777` |
| 🏜️-marrakech-ops | `1489244971772154057` |
| 💻-dev | `1489246953861546115` |

### TEAM
| Channel | Channel ID |
|---------|------------|
| 💬-general | `1489244970983624824` |
| 📅-meetings | `1489244975417000077` |
| 📢-announcements | `1489244980051710162` |

### OGBOUJEE (Jira: OGB) - DIFFERENT BUSINESS
| Channel | Channel ID |
|---------|------------|
| 👜-products | `1489628023266480280` |
| 📦-orders | `1489628028727459945` |
| 👥-clients | `1489628029704736848` |
| 📣-marketing | `1489628033089536162` |
| management | `1490351010252591154` |

### TIKTOK-YOUTUBE (Jira: TIKYT)
| Channel | Channel ID |
|---------|------------|
| 🎥-content | `1489244954646679662` |
| 📆-scheduling | `1489244960245940254` |
| 📊-analytics | `1489244965337956514` |
| 💻-dev | `1489246932629979206` |

### Trading
| Channel | Channel ID |
|---------|------------|
| général | `1498233249925824542` |

### Other
| Channel | Channel ID |
|---------|------------|
| error | `1497426963084611654` |

### Voice Channels
| Channel | Channel ID |
|---------|------------|
| Chill Calls | `1489245009285877951` |
| Meeting Room | `1489245018261557550` |

### Root Channels
| Channel | ID |
|---------|-----|
| Salons textuels (category) | `1489242791849492661` |
| 💻-général | `1489242791849492663` |

### Categories (Parent IDs)
| Category | ID |
|----------|-----|
| Salons textuels | `1489242791849492661` |
| ELIA-HQ | `1489244763017187549` |
| BEN2LUXE | `1489244764808417320` |
| COBOU-AGENCY | `1489244768235028633` |
| ZOVABOOST | `1489244769505906818` |
| TIKTOK-YOUTUBE | `1489244773284974704` |
| MAYAVANTA | `1489244774882873364` |
| TEAM | `1489244778347499691` |
| OGBOUJEE | `1489628000730484938` |
| Trading | `1498233208079257642` |
| **MIRORPAY** | `1506669201715367938` |

# Gmail (server id: gmail — check tools: mcp-cli info gmail search_emails)
mcp-cli call gmail search_emails '{"query":"in:inbox newer_than:7d","maxResults":20}'
mcp-cli call gmail read_email '{"messageId":"..."}'
mcp-cli call gmail draft_email '{"to":["you@example.com"],"subject":"Subject","body":"Plain text body"}'
mcp-cli call gmail send_email '{"to":["you@example.com"],"subject":"Subject","body":"Plain text body"}'
mcp-cli call gmail list_email_labels '{}'

# Gmail-tweetsyncai (server id: gmail-tweetsyncai — tweetsyncai@gmail.com)
## ⚠️ IMPORTANT: This Gmail is for Bene2Luxe Snapchat Ads ONLY!
## Reason: Snapchat Business Ads requires a personal Gmail to create business ads account. Our business emails (contact@bene2luxe.com, contact@cofibou-distribution) cannot create business ads accounts, so we used tweetsyncai@gmail.com for this purpose.
## Use this Gmail MCP for:
## - Bene2Luxe Snapchat Ads management
## - Any business ads-related communications for Bene2Luxe
## - DO NOT use for general emails — use contact@bene2luxe.com or contact@cofibou-distribution for that
mcp-cli call gmail-tweetsyncai search_emails '{"query":"in:inbox newer_than:7d","maxResults":20}'
mcp-cli call gmail-tweetsyncai read_email '{"messageId":"..."}'
mcp-cli call gmail-tweetsyncai send_email '{"to":["you@example.com"],"subject":"Subject","body":"Plain text body"}'

# 📧 IONOS Business Email (PRIMARY - Business Emails)

## Morning Routine Integration

**In the morning routine (MORNING_PROMPT.md), Elia MUST:**
1. Run `python3 /Users/vakandi/EliaAI/tools/get_ide_work.sh` - ALWAYS extract IDE work
2. Check and update Google Calendar with reminders for time-sensitive items
3. Check and update Google Tasks for new todo items
4. After gathering data from all sources (WhatsApp, Discord, Email), sync findings to Google Workspace

## Available MCP Servers

### mail_contact_cofibou_distribution (contact@cofibou-distribution.com)
**Use this for:** Cofibou Distribution LLC emails - business logistics, carrier outreach, Point Relais setup

### mail_contact_cobou_agency (contact@cobou.agency)
**Use this for:** CoBou Agency, ZovaBoost, AccForge emails

### mail_contact_mirorpay_com (contact@mirorpay.com)
**Use this for:** MirorPay business emails — payment notifications, customer inquiries, order confirmations

### yandex-search
**Use this for:** Web search via Yandex Search API — `ai_search_with_yazeka` (AI-generated answers with sources) and `web_search` (simple web results). Requires Yandex API key.

```bash
mcp-cli call yandex-search ai_search_with_yazeka '{"body":{"query":"<your query>","search_region":"en"}}'
mcp-cli call yandex-search web_search '{"body":{"query":"<your query>","search_region":"en"}}'
```

### zernio (Multi-Platform Social Media Engine) — 35 tools
**Use this for:** Posting, scheduling, and managing content across 15+ social platforms via Zernio API.

> ⚠️ **For full command reference, read `ZERNIO_TOOLS.md` in the agent's workspace folder (e.g. `/Users/vakandi/EliaAI/context/ZERNIO_TOOLS.md` for community-organic agent).** That file contains all examples for posting, stories, reels, shorts, TikTok, comments, analytics, and queue scheduling.

**Architecture:** Multi-account with per-account proxies. 3 accounts:
- `account-1` (tweetsyncai) → Instagram, Pinterest
- `account-2` (leitarvisku) → Reddit, YouTube
- `account-3` (waionshredder) → Facebook, LinkedIn

**All tools require `account_id`** (except `zernio_get_accounts` and `zernio_sync_platforms`).

**Media upload:** Local files via `media_paths` (jpg, png, gif, webp, mp4, mov). No third-party hosts needed.

**Quick start:**
```bash
# List accounts
mcp-cli call zernio zernio_get_accounts '{}'

# List connected social accounts (get IDs)
mcp-cli call zernio zernio_list_connected_accounts '{"account_id":"account-1"}'

# Sync platforms (run after connecting new platform)
mcp-cli call zernio zernio_sync_platforms '{}'

# Quick post
mcp-cli call zernio zernio_quick_post '{"account_id":"account-1","content":"Hello!","platform":"instagram","profile_id":"PROFILE_ID","publish_now":true}'

# Story (Instagram/Facebook — requires media)
mcp-cli call zernio zernio_quick_post '{"account_id":"account-1","content":"...","platform":"instagram","profile_id":"PROFILE_ID","publish_now":true,"media_paths":["/path/to/image.jpg"],"platformSpecificData":{"contentType":"story"}}'

# Reel
mcp-cli call zernio zernio_quick_post '{"account_id":"account-1","content":"...","platform":"instagram","profile_id":"PROFILE_ID","publish_now":true,"media_paths":["/path/to/reel.mp4"],"platformSpecificData":{"contentType":"reel"}}'

# YouTube Short (auto-detected: vertical 9:16 ≤3 min)
mcp-cli call zernio zernio_quick_post '{"account_id":"account-2","content":"... #shorts","platform":"youtube","profile_id":"PROFILE_ID","publish_now":true,"media_paths":["/path/to/vertical.mp4"]}'

# TikTok (tiktokSettings required)
mcp-cli call zernio zernio_quick_post '{"account_id":"account-2","content":"...","platform":"tiktok","profile_id":"PROFILE_ID","publish_now":true,"media_paths":["/path/to/video.mp4"],"tiktokSettings":{"privacy_level":"PUBLIC_TO_EVERYONE","allow_comment":true,"allow_duet":true,"allow_stitch":true}}'
```

**Tool categories:** Account & Config (5) · Post Creation (2) · Post Management (9) · Comment Management (8) · Queue Scheduling (5) · Analytics (5)

**Error handling:** `{ success: false, error: "description" }` with `isError: true`. 401/403/429 throw, everything else returns clean error response.

### twitter-mcp (Twitter/X API v2)
**Server:** `twitter-mcp` (already in mcp-cli config — confirmed)
**Tools:** 14 tools available

```bash
# List all tools
mcp-cli info twitter-mcp

# Search tweets (WORKS — no auth needed for search)
mcp-cli call twitter-mcp search '{"query":"Stripe Maroc","limit":10}'

# Post a tweet (⚠️ AUTH FAILS — "BoyerDarci42387" account auth expired)
mcp-cli call twitter-mcp tweet '{"text":"Your tweet content here"}'

# Reply to a tweet
mcp-cli call twitter-mcp reply '{"text":"Reply content","tweet_id":1234567890}'

# Like / Retweet
mcp-cli call twitter-mcp like '{"tweet_id":1234567890}'
mcp-cli call twitter-mcp retweet '{"tweet_id":1234567890}'

# User info
mcp-cli call twitter-mcp get_user '{"screen_name":"elonmusk"}'

# DMs
mcp-cli call twitter-mcp dm '{"text":"Message","user_ids":[123456]}'

# Trends (⚠️ AUTH FAILS — "CottrellDa73338" account auth expired)
mcp-cli call twitter-mcp get_trends '{}'

# Account status check
mcp-cli call twitter-mcp get_account_status '{"username":"someuser"}'
```

**Auth Status (confirmed 2026-07-03):**
| Tool | Status | Error |
|------|--------|-------|
| `search` | ✅ Works | — |
| `get_user` | ❌ Fails | Account not found (API change?) |
| `tweet` | ❌ Fails | Auth failed for BoyerDarci42387 |
| `get_trends` | ❌ Fails | Auth failed for CottrellDa73338 |

→ **Only search works.** Tweet, trends, and user lookup need new API keys. Not blocking for read-only use cases.

## ⚠️ CRITICAL: How to Send Emails (JSON Payload Method)

**The `recipients` field MUST be a list (array), NOT a string!**

### WRONG ❌
```bash
mcp-cli call mail_contact_cofibou_distribution send_email '{"recipients":"recipient@email.com",...}'
```

### CORRECT ✅ — Use Python for complex body text
```python
python3 << 'PYEOF'
import subprocess
import json

body = """Your email body here.
Can have multiple lines."""

payload = {
    "recipients": ["recipient@email.com"],
    "subject": "Subject Line",
    "body": body
}

result = subprocess.run(
    ["mcp-cli", "call", "mail_contact_cofibou_distribution", "send_email", json.dumps(payload)],
    capture_output=True, text=True
)
print(result.stdout)
PYEOF
```

## Quick Commands

```bash
# Read inbox
mcp-cli call mail_contact_cofibou_distribution list_emails_metadata '{"limit":20}'
mcp-cli call mail_contact_cofibou_agency list_emails_metadata '{"limit":20}'

# Read specific email
mcp-cli call mail_contact_cofibou_distribution get_emails_content '{"email_ids":["123"]}'

# Send email (via Python - see above for body with special chars)
```

## ⚠️ IMPORTANT: Use COFIBOU email for business logistics!
- **contact@cofibou-distribution.com** = Cofibou Distribution LLC = business logistics, carriers, Point Relais
- **contact@cobou.agency** = CoBou Agency = CoBou/ZovaBoost/AccForge business

# ⚠️ CRITICAL: Most business emails (orders, invoices, notifications) now redirect to this inbox!
# Check this FIRST for business issues, not Gmail.

mcp-cli call mail_contact_cobou_agency list_available_accounts '{}'
mcp-cli call mail_contact_cobou_agency list_emails_metadata '{"account_name":"ionos","limit":20}'
mcp-cli call mail_contact_cobou_agency get_emails_content '{"account_name":"ionos","email_ids":["123"]}'
mcp-cli call mail_contact_cobou_agency send_email '{"account_name":"ionos","recipients":[" recipient@email.com"],"subject":"Subject","body":"Message"}'
mcp-cli call mail_contact_cobou_agency delete_emails '{"account_name":"ionos","email_ids":["123"]}'
mcp-cli call mail_contact_cobou_agency download_attachment '{"account_name":"ionos","email_id":"123","attachment_index":0,"save_path":"/path/to/save"}'

# Jira
mcp-cli call mcp-atlassian create_issue '{"project":"BEN","summary":"...","description":"...","issue_type":"Task"}'
mcp-cli call mcp-atlassian jira_get_project_issues '{"project_key":"BEN"}'

# Analytics-MCP (Google Analytics 4)
# Service account needs permission in GA4 first!
mcp-cli call analytics-mcp get_account_summaries '{}'
mcp-cli call analytics-mcp get_property_details '{"property_id": "432287363"}'
mcp-cli call analytics-mcp list_ga4_audiences '{"property_id": "432287363"}'
mcp-cli call analytics-mcp list_key_events '{"property_id": "432287363"}'

# Full tool list: See /Users/vakandi/.config/opencode/skills/mcp-cli/SKILL.md

# SSH
mcp-cli call ssh-mpc-server-multisaasdeploy execute-command '{"cmdString":"ls -la"}'
# ⚠️ NEVER USE multisaasdeploy OUTSIDE /Users/vakandi/Documents/MultiSaasDeploy — Wael forbids it anywhere else. Keep the command in TOOLS.md as reference only.
```

### Business Groups (WhatsApp)
| Group | JID |
|-------|-----|
| COBOU PowerRangers | `120363420711538035@g.us` |
| B2LUXE BUSINESS | `120363408208578679@g.us` |
| OGBOUJEE 👜 BUSINESS | `120363425082264099@g.us` |
| MAYAVANTA | `120363405622746597@g.us` |

### WhatsApp Individual IDs (CRITICAL)
| Membre | WhatsApp ID | Téléphone | Pays |
|--------|-----------|---------|------|
| Thomas | 165558221861055 | +33 6 29 35 22 37 | France |
| Rida | 131319447212112 | [À confirmer] | [À confirmer] |
| Ali | 178481677779049 | +41 (Suisse) | ✅ CORRIGÉ |

---

## Agent-Browser (Web Automation)

**Info:** `agent-browser -h`

⚠️ **ALWAYS use `--profile ~/.agent-browser-profile` for logged-in sessions** (saved passwords, cookies, auth). This is the ONLY supported mode. Never use `--executable-path` without `--profile`.

> **🤖 Launchd/Cron/Subworker?** If you're running in a launchd agent, cron job, or subworker context (headless, no GUI), **use `parallel-browser-mcp` instead** — it runs headless natively, maintains session state across calls via native MCP, and supports multiple concurrent sessions. agent-browser requires `--headed` + `--profile` which doesn't work in headless automation contexts.

```bash
# ⚠️ ALWAYS close first, then open with profile + headed mode
agent-browser close && agent-browser --profile ~/.agent-browser-profile --headed open "<url>"

# Navigation & interaction
agent-browser click <selector>
agent-browser fill <selector> <text>
agent-browser snapshot
agent-browser screenshot [path]

# ⚠️ EVAL TOOL - Use for COMPLICATED tasks (better & more robust)
# Execute JavaScript directly in the browser context
agent-browser eval "document.querySelector('selector').click()"
agent-browser eval "Array.from(document.querySelectorAll('.item')).map(el => el.innerText)"
agent-browser eval "localStorage.setItem('key', 'value')"
agent-browser eval "window.scrollTo(0, document.body.scrollHeight)"

# WHY USE EVAL FOR COMPLICATED TASKS:
# ✅ More robust - Direct JavaScript execution avoids selector fragility
# ✅ Better for complex logic - Run conditional logic, loops, data extraction in browser
# ✅ Access browser APIs - localStorage, sessionStorage, window objects
# ✅ Handle dynamic content - Wait for elements, trigger events programmatically
# ✅ Extract structured data - Return complex objects, arrays from page

# Examples for complicated tasks:
agent-browser eval "(function() { 
  const items = []; 
  document.querySelectorAll('.product').forEach(el => { 
    items.push({ 
      name: el.querySelector('h2')?.innerText, 
      price: el.querySelector('.price')?.innerText 
    }); 
  }); 
  return items; 
})()"
```

More options: `agent-browser -h`

---

## 🌐 Parallel Browser MCP (Multi-Session Browser Automation)

**Server:** `parallel-browser-mcp` — Run multiple isolated browser sessions concurrently. Each session gets a numeric ID (`1`, `2`, `3`...) and all `browser_*` tools accept a `sessionId` parameter.

**Providers:** Playwright (local Chromium), Browserbase, Anchor Browser, Cloudflare Browser Run

### ⚠️ CRITICAL: Session Isolation Rules

**Each agent MUST use its own sessions.** Never assume you're the only user. Another agent (cronjob, subworker, manual session) may be using the same MCP server simultaneously.

| Rule | Why |
|------|-----|
| **ALWAYS create your own session** | Never reuse an existing session ID blindly |
| **Name sessions distinctly** | Use a prefix like `{agent}-{task}` so you know which session is yours |
| **Close your session when done** | Don't leave stale sessions consuming resources |
| **Check `get_sessions` first** | See what's running before creating new ones |

### Session Naming Convention

```
{agent_name}_{task_description}
```

Examples:
- `elia_morning_briefing` — Elia cron checking emails
- `bene2_scraping_competitor` — Bene2Luxe product scraping
- `gilfoyle_deploy_test` — Gilfoyle testing a deployment
- `tiktok_video_verify` — TikTok subworker verifying a video

**If you create a session without a distinct name, you risk colliding with another agent's work.**

### ⚠️ mcp-cli Limitation

**`mcp-cli call` spawns a new process per call → session state is lost between calls.** This server is designed for **persistent MCP connections** (OpenCode native, Claude Code, Cursor). 

- ✅ **Works natively in OpenCode** — use `tool_call` directly (sessions persist across calls)
- ✅ **Works in single process** — Node.js scripts that maintain one MCP connection
- ❌ **Does NOT work with `mcp-cli call`** — each call is a fresh process, no session persistence

For quick checks only: `mcp-cli call parallel-browser-mcp get_sessions '{}'` (works because no state needed).

### Workflow (via OpenCode native MCP — verified working)

```
# 1. CHECK what sessions exist
parallel-browser-mcp_get_sessions()

# 2. CREATE your session with a distinct name
parallel-browser-mcp_start_session({"provider":"playwright","name":"gilfoyle_test"})
# Returns: {"id": 1, "provider": "playwright", ...}

# 3. USE the session with sessionId
parallel-browser-mcp_browser_navigate({"sessionId":1,"url":"https://example.com"})
# Returns: {"url":"https://example.com/","status":200,"title":"Example Domain"}

parallel-browser-mcp_browser_snapshot({"sessionId":1})
# Returns: {"title":"Example Domain","url":"https://example.com/","tree":[...]}

parallel-browser-mcp_browser_click({"sessionId":1,"selector":"#login"})
parallel-browser-mcp_browser_fill({"sessionId":1,"selector":"#email","value":"test@example.com"})
parallel-browser-mcp_browser_screenshot({"sessionId":1})
parallel-browser-mcp_browser_evaluate({"sessionId":1,"script":"document.title"})

# 4. CLOSE your session when done
parallel-browser-mcp_close_session({"sessionId":1})
# Returns: {"sessionId":1,"closed":true}
```

### Available Tools (26 tools)

| Category | Tools |
|----------|-------|
| **Session Management** | `start_session`, `close_session`, `close_all_sessions`, `get_sessions` |
| **Navigation** | `browser_navigate`, `browser_go_back` |
| **Interaction** | `browser_click`, `browser_fill`, `browser_fill_form`, `browser_hover`, `browser_select_option`, `browser_upload_file` |
| **Capture** | `browser_screenshot`, `browser_snapshot`, `browser_get_page_structure`, `browser_dom_query` |
| **Input** | `browser_keyboard_press`, `browser_keyboard_type`, `browser_mouse_move`, `browser_mouse_click_xy`, `browser_mouse_drag`, `browser_drag` |
| **Utility** | `browser_evaluate`, `browser_generate_locator`, `browser_wait_for_selector`, `browser_wait_for_timeout` |

### When to Use Parallel Browser vs Agent-Browser

| Scenario | Use |
|----------|-----|
| Single browsing task, needs logged-in session (cookies, passwords) | **agent-browser** (`--profile ~/.agent-browser-profile`) |
| Multiple concurrent browsing tasks (scraping, monitoring, verification) | **parallel-browser-mcp** |
| Need stealth/anti-detection (CloakBrowser) | **parallel-browser-mcp** with `useCloakBrowser: true` |
| Quick one-off page check | **agent-browser** (simpler) |
| **🤖 Launchd/cron/subworker (headless automation)** | **parallel-browser-mcp** ← PRIORITIZE THIS |
| Running in a workspace with no GUI | **parallel-browser-mcp** ← PRIORITIZE THIS |

---

## Voice Transcription (Whisper)

**Info:** `whisper -h`

```bash
# Download audio first, then transcribe
whisper /path/to/audio.ogg --model large-v3 --language French --task transcribe
```

---

## IDE Work Extraction

**Info:** `./tools/get_ide_work.sh -h`

```bash
./tools/get_ide_work.sh              # All IDEs (recommended)
./tools/get_opencode_work.sh 24       # OpenCode only (last 24h)
```

---

## Google Workspace Calendar (Meeting Scheduling)

**Info:** `gws-workspace -h` or `gws-workspace help`

```bash
gws-workspace create-event "Title" "Desc"            # Calendar event without Meet
gws-workspace create-meet "Title" "Desc"              # Calendar event WITH Google Meet link
gws-workspace create-meet "Title" "Desc" "2026-05-10T15:30:00" "2026-05-10T16:30:00"  # + start/end times
gws-workspace create-task "Task" "Notes"
gws-workspace import-md "file.md" "Title"
gws-workspace list-events
```

**📅 MEETING SCHEDULING - Elia MUST follow this workflow:**

### For Discord Team Meetings (when Wael asks for a meeting):
**→ Use Discord Server Events (so all team members can see)**
```bash
# Create Discord scheduled event (visible to all team members)
mcp-cli call discord-server-mcp discord_execute '{
  "operation": "events.create",
  "params": {
    "name": "Weekly Team Sync",
    "description": "Team sync meeting with Wael",
    "start_time": "2026-04-25T10:00:00Z",
    "end_time": "2026-04-25T10:30:00Z",
    "location": "Voice Channel: Meeting Room"
  }
}'
```

**Example:**
```bash
# Create team meeting visible in Discord
mcp-cli call discord-server-mcp discord_execute '{"operation":"events.create","params":{"name":"Team Sync with Wael","description":"Weekly sync - discuss current projects","start_time":"2026-04-28T10:00:00Z","end_time":"2026-04-28T10:30:00Z","location":"Meeting Room"}}'
```

### For External Meetings or Personal Tasks:
**→ Use Google Calendar with Google Meet link**
```bash
# Create Google Calendar event WITH Google Meet link (preferred)
gws-workspace create-meet "Meeting Title" "Description"

# With specific start/end times:
gws-workspace create-meet "Meeting Title" "Desc" "2026-05-10T15:30:00" "2026-05-10T16:30:00"

# Create event without Meet (basic)
gws-workspace create-event "Meeting Title" "Description"
```

**Note:** `create-meet` creates a Calendar event + auto-generates a Google Meet link (returns both the Calendar URL and the `meet.google.com/xxx` link).

### Summary:
| Type of Meeting | Where to Create | Tool |
|----------------|-----------------|------|
| Discord Team Meeting (Wael asks) | Discord Server | `discord-server-mcp events.create` |
| External/Personal Meeting | Google Calendar | `gws-workspace create-meet` (with Meet link) or `gws-workspace create-event` |

---

## Jira Projects
| Business | Key |
|----------|-----|
| Bene2Luxe | `BEN` |
| CoBou Agency | `COBOUAGENC` |
| TikTok/YouTube | `TIKYT` |
| ZovaBoost | `ZOVAPANEL` |

---

## 🚀 Coolify CLI & MCP (Self-Hosted PaaS)

### Coolify CLI (installed at ~/.local/bin/coolify)

**Version:** 1.6.2

```bash
# Configure context (run once per instance)
~/.local/bin/coolify context add \
  --name production \
  --url https://dashboard.cobou.agency \
  --token YOUR_COOLIFY_API_TOKEN

# List projects
~/.local/bin/coolify project list

# List applications in a project
~/.local/bin/coolify app list --project <project-id>

# Deploy an application
~/.local/bin/coolify deploy <application-uuid>

# Delete an application
~/.local/bin/coolify app delete <application-uuid>

# List servers
~/.local/bin/coolify server list

# List services
~/.local/bin/coolify service list

# Output as JSON for scripting
~/.local/bin/coolify project list --format json
```

### Coolify MCP (38 tools via mcp-cli)

**Server:** dashboard.cobou.agency (Coolify v4.0.0, @masonator/coolify-mcp v2.10.0)

```bash
# List all MCP servers (check if coolify is available)
mcp-cli

# Quick commands
mcp-cli call coolify get_version
mcp-cli call coolify projects '{"action":"list"}'
mcp-cli call coolify list_applications
mcp-cli call coolify deploy '{"tag_or_uuid":"app-uuid"}'
```

### When to use which

| Task | Use |
|---|---|
| Quick project/app listing | `coolify CLI` |
| Deploy/delete operations | `coolify CLI` |
| Complex queries with filters | `mcp-cli call coolify` |
| Scripting/automation | `coolify CLI --format json` |
| Diagnostics (find_issues, diagnose_app) | `mcp-cli call coolify` |
| Database/service management | `mcp-cli call coolify` |

### REST API (when MCP tools not enough) — PREFERRED FALLBACK

**⚠️ IMPORTANT: Quand le MCP Coolify n'a pas le tool nécessaire, utilise DIRECTEMENT l'API REST Admin. La clé API est dans `~/.config/mcp/`.**

Ne perds pas de temps à chercher des workarounds MCP — va direct à l'API REST. Le token admin est dans `/Users/vakandi/.config/mcp/mcp_servers.json`.

```bash
TOKEN="<COOLIFY_ACCESS_TOKEN de ~/.config/mcp/mcp_servers.json>"
API="https://dashboard.cobou.agency/api/v1"

# Application status
curl -s "$API/applications/$UUID" -H "Authorization: Bearer $TOKEN"

# Deployment details
curl -s "$API/deployments/$DEPLOY_UUID" -H "Authorization: Bearer $TOKEN"

# List deployments for app
curl -s "$API/deployments?resource_id=$UUID&limit=5" -H "Authorization: Bearer $TOKEN"

# Environment variables
curl -s "$API/applications/$UUID/envs" -H "Authorization: Bearer $TOKEN"

# Deploy logs for specific resource
curl -s "$API/deployments?resource_id=$UUID" -H "Authorization: Bearer $TOKEN"
```

### Quick Commands (MCP)

```bash
# Version & Health
mcp-cli call coolify get_version
mcp-cli call coolify get_mcp_version
mcp-cli call coolify get_infrastructure_overview

# Diagnostics
mcp-cli call coolify find_issues
mcp-cli call coolify diagnose_app '{"query":"app-uuid-or-name"}'
mcp-cli call coolify diagnose_server '{"query":"server-uuid-or-ip"}'

# Servers
mcp-cli call coolify list_servers '{"page":1,"per_page":20}'
mcp-cli call coolify get_server '{"uuid":"server-uuid"}'
mcp-cli call coolify validate_server '{"uuid":"server-uuid"}'

# Projects & Environments
mcp-cli call coolify projects '{"action":"list"}'
mcp-cli call coolify projects '{"action":"create","name":"MyProject","description":"..."}'
mcp-cli call coolify environments '{"action":"list","project_uuid":"proj-uuid"}'

# Applications
mcp-cli call coolify list_applications
mcp-cli call coolify get_application '{"uuid":"app-uuid"}'
mcp-cli call coolify application '{"action":"create","project_uuid":"...","server_uuid":"...","name":"my-app","build_pack":"docker_image","docker_registry_image_name":"directus/directus","fqdn":"app.domain.com"}'
mcp-cli call coolify application_logs '{"uuid":"app-uuid","lines":50}'

# Databases (PostgreSQL, MySQL, MariaDB, MongoDB, Redis, etc.)
mcp-cli call coolify list_databases
mcp-cli call coolify get_database '{"uuid":"db-uuid"}'
mcp-cli call coolify database '{"action":"create","type":"postgresql","server_uuid":"...","project_uuid":"...","name":"mydb","postgres_user":"directus","postgres_password":"securepass","postgres_db":"directus"}'

# Services (Docker Compose)
mcp-cli call coolify list_services
mcp-cli call coolify get_service '{"uuid":"svc-uuid"}'
mcp-cli call coolify service '{"action":"create","type":"docker-compose","server_uuid":"...","project_uuid":"...","name":"my-service","docker_compose_raw":"version:\\n  services:\\n    ..."}'

# Control (start/stop/restart)
mcp-cli call coolify control '{"resource":"application","action":"restart","uuid":"app-uuid"}'

# Environment Variables
mcp-cli call coolify env_vars '{"resource":"application","action":"list","uuid":"app-uuid"}'
mcp-cli call coolify env_vars '{"resource":"application","action":"create","uuid":"app-uuid","key":"DB_HOST","value":"coolify.internal"}'
mcp-cli call coolify env_vars '{"resource":"application","action":"update","uuid":"app-uuid","env_uuid":"...","value":"new-value"}'

# Deployments
mcp-cli call coolify deploy '{"tag_or_uuid":"app-uuid","force":false}'
mcp-cli call coolify list_deployments
mcp-cli call coolify deployment '{"action":"get","uuid":"deploy-uuid","lines":100}'

# Teams
mcp-cli call coolify teams '{"action":"get_current"}'

# Search Docs
mcp-cli call coolify search_docs '{"query":"how to deploy directus","limit":5}'
```

### Full Tool List (38 tools)

| Category | Tools |
|----------|-------|
| **Infrastructure** | `get_version`, `get_mcp_version`, `get_infrastructure_overview` |
| **Diagnostics** | `diagnose_app`, `diagnose_server`, `find_issues` |
| **Batch Ops** | `restart_project_apps`, `bulk_env_update`, `stop_all_apps`, `redeploy_project` |
| **Servers** | `list_servers`, `get_server`, `validate_server`, `server_resources`, `server_domains` |
| **Projects** | `projects` (list/get/create/update/delete) |
| **Environments** | `environments` (list/get/create/delete) |
| **Applications** | `list_applications`, `get_application`, `application` (create/update/delete), `application_logs` |
| **Databases** | `list_databases`, `get_database`, `database` (create 8 types, delete), `database_backups` (CRUD) |
| **Services** | `list_services`, `get_service`, `service` (create/update/delete with docker-compose) |
| **Control** | `control` (start/stop/restart apps, databases, services) |
| **Env Vars** | `env_vars` (CRUD for app/service env vars) |
| **Deployments** | `list_deployments`, `deploy`, `deployment` (get/cancel/list_for_app) |
| **Private Keys** | `private_keys` (list/get/create/update/delete) |
| **GitHub Apps** | `github_apps` (list/get/create/update/delete) |
| **Teams** | `teams` (list/get/members/current) |
| **Cloud Tokens** | `cloud_tokens` (Hetzner/DigitalOcean: CRUD + validate) |
| **Documentation** | `search_docs` (full-text search Coolify docs) |


---

## 🤖 Telegram Scraper MCP (mirrorpay-telegram)

**Server**: `telegram-scraper` — Python/FastMCP server for Telegram channel discovery, member scraping, and mass invitation.
**Location**: `/Users/vakandi/Documents/mcps_server/telegram-scraper-mcp/server.py`
**Config**: `~/.config/mcp/mcp_servers.json` (timeout: 5h = 18000s)

### Architecture — 2 Types de Comptes

| Type | Index | Rôle | Opérations |
|------|-------|------|------------|
| **Principal** (Wael) | 0 | Discovery safe | `discover_similar_channels`, `join_group`, `get_my_channels` |
| **Scraper** (achetés) | 1+ | Scraping risqué + DM | `scrape_members`, `invite_members`, `bulk_collect`, **`send_dm`** |

### Tools

```bash
# 1. LISTER mes channels/groupes (trouver des seeds)
mcp-cli call telegram-scraper get_my_channels '{"account_index":0,"filter_type":"channels"}'
mcp-cli call telegram-scraper get_my_channels '{"account_index":0,"filter_type":"groups"}'

# 2. DÉCOUVRIR des channels similaires (safe, compte Wael)
# Utilise GetChannelRecommendations — ne marche que pour les GROS channels (10k+ membres)
mcp-cli call telegram-scraper discover_similar_channels '{"seed_channels":["https://t.me/cointelegraph","https://t.me/producthunt","https://t.me/telegram"],"max_results":20}'

# Seeds qui marchent bien:
#   @cointelegraph → 10 similaires (Binance Announcements, Whale Alert, Watcher Guru)
#   @telegram      → 10 similaires (Telegram Tips, Pavel Durov, Toncoin)
#   @notcoin       → 10 similaires (DOGS, CATS, Wallet News, Yescoin)
#   @producthunt   → 10 similaires (Venture Capital, Crypto Insider)
#   @washingtonpost, @nytimes → news channels

# 3. SCRAPER les membres (risqué — utilise compte scraper index 1+)
mcp-cli call telegram-scraper scrape_members '{"channel_url":"https://t.me/notcoin","max_members":100,"account_index":1}'

# 4. INVITER des membres (très risqué — compte scraper)
mcp-cli call telegram-scraper invite_members '{"target_channel":"https://t.me/mon_channel","max_invites":20,"speed":"slow","account_index":1}'
# Speeds: "normal" (15-30s), "slow" (30-45s), "very_slow" (45-74s)

# 5. PIPELINE COMPLET (découvrir → rejoindre → scraper)
mcp-cli call telegram-scraper bulk_collect '{"seed_channels":["https://t.me/cointelegraph","https://t.me/notcoin"],"member_limit_per_channel":200,"max_similar_to_scan":10,"account_index":1}'

# 6. ENVOYER UN DM (parler aux prospects avec un vrai compte, pas un bot)
# Utilise un compte scraper (index 1+ par défaut) — PAS le bot Telegram MCP
mcp-cli call telegram-scraper send_dm \
  '{"recipient":"@username","message":"Ton message perso","account_index":1}'

# Avec lien d'invitation vers le channel MirrorPay
mcp-cli call telegram-scraper send_dm \
  '{"recipient":"@username","message":"Message","invite_link":"https://t.me/MirrorPaySaaS","account_index":1}'

# 7. OUTILS UTILITAIRES
mcp-cli call telegram-scraper list_accounts '{}'        # Lister les comptes configurés
mcp-cli call telegram-scraper get_stats '{}'             # Stats globales + action log
mcp-cli call telegram-scraper join_group '{"group_url":"https://t.me/mon_groupe","account_index":0}'  # Rejoindre un groupe
```

### ⚠️ Anti-Ban Rules

| Règle | Détail |
|-------|--------|
| **Compte Wael** (index 0) | Jamais de scraping ou d'invite — découverte seulement |
| **Comptes scraper** (index 1+) | Pour le scraping et l'invitation |
| **FloodWait** | Attente automatique (pause jusqu'à reprise) |
| **InviteHistory** | `history/invite_history.json` — ne ré-invite jamais deux fois |
| **Délais variables** | 15-45s entre chaque invite selon la vitesse |
| **Privacy filter** | Saute les membres non-invitables automatiquement |

### Action Log (JSONL)

Toutes les actions sont loguées dans `history/actions.jsonl` (un JSON par ligne) :

```bash
# Voir les logs d'action
cat /Users/vakandi/Documents/mcps_server/telegram-scraper-mcp/history/actions.jsonl | python3 -m json.tool --no-ensure-ascii

# Exemple de log:
# {"timestamp":"2026-05-20T12:20:39","action":"discover_similar_channels","params":{...},"duration_ms":10760,"status":"success","result":{"total_similar_found":33}}
```

### Workflow Recommandé

```
1. get_my_channels → trouver des seeds (channels où je suis déjà)
2. discover_similar_channels → trouver des channels similaires (GROS seeds seulement)
3. scrape_members → scraper les membres (compte scraper, index 1+)
4. invite_members → inviter vers notre channel (compte scraper, speed=slow)
```

---


## 🎬 Video Analyzer MCP (custom — ffmpeg + Whisper)

**Server** : `video-analyzer` — Analyse locale de vidéos : métadonnées, frames clés, transcription audio (Whisper), qualité.
**Code** : `/Users/vakandi/Documents/mcps_server/video-analyzer-mcp/server.py` (Python, MCP stdio)
**Dépendances** : `ffmpeg` (brew) + `whisper` (pip — déjà installé)

### 2 Tools

```bash
# 1. ANALYSE COMPLÈTE — métadonnées + frames + transcription + qualité
mcp-cli call video-analyzer analyze_video \
  '{"url":"file:///Users/vakandi/Documents/HiggsFieldGenerator/output/video_20260520.mp4"}'

# Avec options (désactiver transcription, nombre de frames)
mcp-cli call video-analyzer analyze_video \
  '{"url":"file:///path/to/video.mp4","options":{"extract_frames":true,"transcribe":false,"num_frames":3}}'

# 2. MÉTADONNÉES RAPIDES (pas d'analyse lourde)
mcp-cli call video-analyzer get_metadata \
  '{"url":"file:///path/to/video.mp4"}'
```

### Ce que retourne `analyze_video`
| Champ | Description |
|-------|-------------|
| **metadata** | Durée, résolution, fps, codec, has_audio, aspect_ratio |
| **frames[]** | Liste de N frames clés (timestamp, chemin fichier JPG) |
| **transcription** | Texte parlé avec timestamps + segments (via Whisper) |
| **quality_check** | `passed: true/false` + liste de défauts détectés |
| **file_size_mb** | Taille du fichier |

### Qualité automatique
Le serveur détecte automatiquement :
- ❌ Ratio != 9:16 (TikTok vertical)
- ❌ Résolution < 720p
- ❌ Durée < 5s (trop court) ou > 120s (trop long pour TikTok)

### Utilisation dans le subworker TikTok

```bash
# WORKFLOW VÉRIFICATION VIDÉO
# 1. Générer la vidéo (agent-browser + Higgsfield)
# 2. Analyser
mcp-cli call video-analyzer analyze_video '{"url":"file:///path/to/output.mp4"}'
# 3. Lire quality_check.passed → si false, régénérer
# 4. Lire transcription.text → vérifier le contenu parlé
# 5. Lire metadata → vérifier ratio 9:16, résolution, durée
```

---

## 🔍 GSC MCP (Google Search Console — 57 tools)

**Server name:** `gsc-mcp`
**Multi-site:** Pass `account="mirorpay"` for MirorPay, `account="bene2luxe"` for Bene2Luxe. Default (no account) = mirorpay.

**All tools accept optional `account` param** for multi-site switching. Always use sc-domain format for site URLs: `sc-domain:mirorpay.com` (NOT `sc_domain:`).

```bash
# Get site details (check permissions)
mcp-cli call gsc-mcp get_site_details '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# List all tracked properties
mcp-cli call gsc-mcp list_properties '{"account":"mirorpay"}'
```

### 📊 Search Analytics (8 tools)

```bash
# Core search analytics (top pages/queries by clicks, impressions, CTR, position)
mcp-cli call gsc-mcp get_search_analytics '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Performance overview with totals + daily breakdown
mcp-cli call gsc-mcp get_performance_overview '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Compare two periods with absolute/relative deltas
mcp-cli call gsc-mcp compare_search_periods '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Advanced analytics with multi-dimension breakdown
mcp-cli call gsc-mcp get_advanced_search_analytics '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Anomaly detection (spikes/drops in queries)
mcp-cli call gsc-mcp analytics_anomalies '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Search type breakdown (Web, Image, Video, News, Discover)
mcp-cli call gsc-mcp search_type_breakdown '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Query by page (what queries drove traffic to a specific page)
mcp-cli call gsc-mcp get_search_by_page_query '{"site_url":"sc-domain:mirorpay.com","page":"/","account":"mirorpay"}'

# Discover + News performance
mcp-cli call gsc-mcp discover_performance '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'
mcp-cli call gsc-mcp news_performance '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'
```

### 🚨 SEO Alerts & Opportunities (6 tools)

```bash
# Quick SEO wins (pages needing title/meta/desc improvements)
# NOTE: tool parameter is "site", NOT "site_url"
mcp-cli call gsc-mcp quick_wins '{"site":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Detect traffic drops by page
mcp-cli call gsc-mcp traffic_drops '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Google Search Console alerts
mcp-cli call gsc-mcp check_alerts '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Striking distance queries (positions 4-15, close to top 3)
mcp-cli call gsc-mcp seo_striking_distance '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Content cannibalization detection
mcp-cli call gsc-mcp seo_cannibalization '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Lost queries (dropped from results)
mcp-cli call gsc-mcp seo_lost_queries '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'
```

### 🔎 URL Inspection & Indexing (7 tools)

**IMPORTANT:** `inspect_url` uses `site` + `url` (NOT `site_url` + `inspection_url`).
`submit_url` / `submit_batch` use `url`/`urls` only (no `site` param).
`force_reindex` uses `site` + `urls`.
`check_indexing_issues` uses `site` + `urls`.

```bash
# Inspect URL — returns 13 fields from Google: verdict, coverage_state, robots_txt_state,
# indexing_state, last_crawl, page_fetch_state, google_canonical, user_canonical,
# sitemap[], referring_urls[], crawled_as, mobile_verdict, gsc_inspection_link, category
mcp-cli call gsc-mcp inspect_url '{"site":"sc-domain:mirorpay.com","url":"https://mirorpay.com/","account":"mirorpay"}'

# Batch URL inspection (max 10)
mcp-cli call gsc-mcp batch_url_inspection '{"site":"sc-domain:mirorpay.com","urls":["https://mirorpay.com/","https://mirorpay.com/blog"],"account":"mirorpay"}'

# Indexing issues — returns only non-indexed URLs with summary by category
# Categories: indexed, not_indexed, unknown, discovered_not_indexed, crawled_not_indexed,
#   duplicate, excluded, robots_blocked, fetch_error, canonical_issue
mcp-cli call gsc-mcp check_indexing_issues '{"site":"sc-domain:mirorpay.com","urls":["https://mirorpay.com/"],"account":"mirorpay"}'

# Submit URL to Google Indexing API (requires Indexing API enabled)
# Returns: status, notify_time, latest_update_type, first_publishing_time
mcp-cli call gsc-mcp submit_url '{"url":"https://mirorpay.com/blog/new-post","account":"mirorpay"}'
mcp-cli call gsc-mcp submit_batch '{"urls":["https://mirorpay.com/blog/post1","https://mirorpay.com/blog/post2"],"account":"mirorpay"}'

# Force reindex: inspects + auto-submits non-indexed URLs
# Full inspection data per URL (coverage_state, sitemap, referring_urls, etc.)
mcp-cli call gsc-mcp force_reindex '{"site":"sc-domain:mirorpay.com","urls":["https://mirorpay.com/pricing"],"account":"mirorpay"}'
```

### 🗺️ Sitemap Management (5 tools)

```bash
# List all sitemaps
mcp-cli call gsc-mcp list_sitemaps '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Get specific sitemap details
mcp-cli call gsc-mcp sitemaps_get '{"site_url":"sc-domain:mirorpay.com","feedpath":"https://mirorpay.com/sitemap.xml","account":"mirorpay"}'

# Submit sitemap
mcp-cli call gsc-mcp submit_sitemap '{"site_url":"sc-domain:mirorpay.com","feedpath":"https://mirorpay.com/sitemap.xml","account":"mirorpay"}'

# Delete sitemap
mcp-cli call gsc-mcp sitemaps_delete '{"site_url":"sc-domain:mirorpay.com","feedpath":"https://mirorpay.com/sitemap.xml","account":"mirorpay"}'

# Full sitemap audit (cross-reference sitemap URLs vs GSC indexed)
mcp-cli call gsc-mcp sitemap_audit '{"site_url":"sc-domain:mirorpay.com","sitemap_url":"https://mirorpay.com/sitemap.xml","account":"mirorpay"}'
```

### 📈 GA4 Integration (7 tools)

```bash
# Organic landing pages from GA4
mcp-cli call gsc-mcp ga4_organic_landing_pages '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Traffic sources (channel, source, medium)
mcp-cli call gsc-mcp ga4_traffic_sources '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Page performance (views, users, engagement, conversions, revenue)
mcp-cli call gsc-mcp ga4_page_performance '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Real-time active users
mcp-cli call gsc-mcp ga4_realtime '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# User behavior (device, country, new vs returning)
mcp-cli call gsc-mcp ga4_user_behavior '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Conversion funnel (event sequence)
mcp-cli call gsc-mcp ga4_conversion_funnel '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Full funnel analysis (requires steps param)
mcp-cli call gsc-mcp ga4_funnel '{"site_url":"sc-domain:mirorpay.com","steps":["page_view","session_start","purchase"],"account":"mirorpay"}'
```

### 🔗 Cross GSC+GA4 Analysis (2 tools)

```bash
# Traffic health check (combined GSC impressions + GA4 sessions)
mcp-cli call gsc-mcp traffic_health_check '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Page-level analysis (GSC ranking + GA4 engagement)
mcp-cli call gsc-mcp page_analysis '{"site_url":"sc-domain:mirorpay.com","page":"/","account":"mirorpay"}'
```

### 📋 Technical Audits (9 tools)

```bash
# Page health score (LCP, FID, CLS, mobile-friendliness)
mcp-cli call gsc-mcp page_health_score '{"site_url":"sc-domain:mirorpay.com","page_url":"https://mirorpay.com","account":"mirorpay"}'

# Full page technical audit (9 issue categories)
mcp-cli call gsc-mcp page_technical_audit '{"site_url":"sc-domain:mirorpay.com","page_url":"https://mirorpay.com","account":"mirorpay"}'

# AI crawler visibility audit (9 crawlers: GPTBot, Claude, etc.)
mcp-cli call gsc-mcp ai_visibility_audit '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Schema validation
mcp-cli call gsc-mcp schema_validate '{"site_url":"sc-domain:mirorpay.com","url":"https://mirorpay.com","account":"mirorpay"}'

# Schema generation (Article, Product, FAQPage, LocalBusiness)
mcp-cli call gsc-mcp schema_generate '{"site_url":"sc-domain:mirorpay.com","page_url":"https://mirorpay.com/blog/post","page_type":"article","account":"mirorpay"}'

# Content quality analysis
mcp-cli call gsc-mcp content_quality '{"site_url":"sc-domain:mirorpay.com","url":"https://mirorpay.com","account":"mirorpay"}'

# Content brief (keyword research + GA4 data)
mcp-cli call gsc-mcp content_brief '{"site_url":"sc-domain:mirorpay.com","url":"https://mirorpay.com/blog","account":"mirorpay"}'

# Hreflang audit
mcp-cli call gsc-mcp hreflang_audit '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Preload resource audit
mcp-cli call gsc-mcp preload_audit '{"site_url":"sc-domain:mirorpay.com","url":"https://mirorpay.com","account":"mirorpay"}'
```

### 📉 CrUX (3 tools — need CRUX_API_KEY env var)

```bash
mcp-cli call gsc-mcp crux_page_vitals '{"site_url":"sc-domain:mirorpay.com","url":"https://mirorpay.com","account":"mirorpay"}'
mcp-cli call gsc-mcp crux_history '{"site_url":"sc-domain:mirorpay.com","url":"https://mirorpay.com","account":"mirorpay"}'
mcp-cli call gsc-mcp crux_lcp_subparts '{"site_url":"sc-domain:mirorpay.com","url":"https://mirorpay.com","account":"mirorpay"}'
```

### 🔄 Drift Analysis (3 tools)

```bash
# Create baseline from two date ranges
mcp-cli call gsc-mcp drift_baseline '{"site_url":"sc-domain:mirorpay.com","baseline_start":"90daysAgo","baseline_end":"today","account":"mirorpay"}'

# Compare with drift
mcp-cli call gsc-mcp drift_compare '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'

# Get drift history
mcp-cli call gsc-mcp drift_history '{"site_url":"sc-domain:mirorpay.com","account":"mirorpay"}'
```

### 🧪 Other Tools (3 tools)

```bash
# Parasite SEO risk assessment
mcp-cli call gsc-mcp parasite_risk '{"site_url":"sc-domain:mirorpay.com","urls":["https://mirorpay.com/path1","https://mirorpay.com/path2"],"account":"mirorpay"}'

# Pagespeed audit (requires GOOGLE_API_KEY env var)
mcp-cli call gsc-mcp pagespeed_audit '{"site_url":"sc-domain:mirorpay.com","url":"https://mirorpay.com","account":"mirorpay"}'

# IndexNow URL submission (needs site + key + urls)
mcp-cli call gsc-mcp indexnow_submit '{"site_url":"sc-domain:mirorpay.com","site":"https://mirorpay.com","key":"your-indexnow-key","urls":["https://mirorpay.com/new-page"],"account":"mirorpay"}'

# GBP deprecation lint (checks if pages rely on Google Business Profile)
mcp-cli call gsc-mcp gbp_deprecation_lint '{"site_url":"sc-domain:mirorpay.com","url":"https://mirorpay.com","account":"mirorpay"}'

# Get MCP capabilities (list all 57 tools)
mcp-cli call gsc-mcp get_capabilities '{}'
```

### 📝 Known Issues

| Tool | Status | Issue |
|------|--------|-------|
| `crux_*` (3 tools) | ❌ Needs env var | `CRUX_API_KEY` not set |
| `pagespeed_audit` | ❌ Needs env var | `GOOGLE_API_KEY` not set |
| `indexnow_submit` | ⚠️ Needs config | IndexNow key + site |
| `ga4_funnel` | ⚠️ Needs params | `steps` array required |
| Bene2Luxe GA4 | ⚠️ 403 | Service account lacks GA4 access for property 465289591 |

---

## 🎬 HeyGen Content MCP (19 tools — Local-First Video Pipeline)

**Server:** `heygen-content-mcp`
**Location:** `/Users/vakandi/Documents/mcps_server/heygen-content-mcp/`
**Architecture:** 100% local — no HeyGen API key needed

**Engines:** HyperFrames Whisper (transcribe), Kokoro-82M (TTS), Playwright+FFmpeg (render), FFmpeg (clip/cut/overlay), OpenRouter (AI analysis)

### Quick Start

```bash
# Transcribe a video
mcp-cli call heygen-content-mcp transcribe '{"file_path":"/path/to/video.mp4","model":"medium.en"}'

# AI finds best moments from transcript
mcp-cli call heygen-content-mcp analyze_transcript '{"transcript":"transcript.json","max_clips":10}'

# Cut multiple clips at once
mcp-cli call heygen-content-mcp cut_multiple_clips '{"video_path":"/path/to/video.mp4","windows":[{"start":120,"end":180,"label":"funny_moment"}]}'

# Create vertical 9:16 clip for TikTok/Reels
mcp-cli call heygen-content-mcp create_vertical_clip '{"video_path":"/path/to/video.mp4","start":120,"end":180,"add_captions":true,"transcript_path":"transcript.json"}'

# Add branding text overlay
mcp-cli call heygen-content-mcp add_text_overlay '{"video_path":"clip.mp4","text":"YOUR BRAND","position":"bottom","font_size":36}'

# Generate TTS voiceover
mcp-cli call heygen-content-mcp tts '{"text":"Welcome!","voice":"af_heart","output_filename":"intro.wav"}'
```

### Full Documentation

📖 **Complete tool reference, workflows, and examples:**
`/Users/vakandi/EliaAI/context/HEYGEN_TOOLS.md`

This document covers:
- All 19 tools with parameters and examples
- Step-by-step workflow: edit video from podcast/competitor content
- Complete examples for repurposing content
- Tips & best practices for each tool

---

## 📈 Google Trends MCP (Google Trends Data — 9 tools)

**Server name:** `google-trends`
**Location:** `/Users/vakandi/Documents/mcps_server/GoogleTrendsMCP/`
**Library:** `pytrends-modern` (modern fork of pytrends — no API key needed)
**Rate limits:** Unofficial — wait 60s on 429 errors. Max ~10-15 queries/session recommended.

### ⚠️ Rate Limit Strategy

Each tool returns MAXIMUM data per call to minimize API calls. Use `full_keyword_research` to get everything about a keyword in ONE call (time series + related queries + topics + regions + suggestions).

### 9 Tools

#### 🔍 Research (1 call = everything about a keyword)

```bash
# FULL KEYWORD RESEARCH — one call dumps everything (time series, related queries, topics, regions, suggestions)
# USE THIS FIRST to minimize API calls under rate limits
mcp-cli call google-trends full_keyword_research '{"keyword":"Stripe alternative","timeframe":"today 12-m","geo":"FR"}'
```

#### 📊 Compare Keywords

```bash
# Compare up to 5 keywords — returns time series + summary stats (mean, max, trend direction)
mcp-cli call google-trends compare_keywords '{"keywords":["Stripe alternative","payment gateway ban","high risk merchant"],"timeframe":"today 12-m","geo":"FR"}'
```

#### 🔥 Trending Searches

```bash
# Today's trending searches by country (daily snapshot)
mcp-cli call google-trends get_trending_searches '{"country":"france"}'

# Real-time trending (RSS-powered, includes images/news)
mcp-cli call google-trends get_realtime_trending '{"country":"FR","count":100}'
```

#### 🔗 Related Queries & Topics

```bash
# Related queries — top + rising (full tables with value scores)
mcp-cli call google-trends get_related_queries '{"keyword":"Stripe ban","timeframe":"today 12-m","geo":"FR"}'

# Related topics — thematic connections
mcp-cli call google-trends get_related_topics '{"keyword":"payment gateway","timeframe":"today 12-m"}'

# Autocomplete suggestions (long-tail variations)
mcp-cli call google-trends get_suggestions '{"keyword":"payment cloaking"}'
```

#### 🌍 Regional Interest

```bash
# Where a keyword is most popular — ALL regions with geo codes, sorted
mcp-cli call google-trends get_interest_by_region '{"keyword":"Stripe alternative","timeframe":"today 12-m","geo":"FR","resolution":"REGION"}'
```

#### 📈 Trending Analysis

```bash
# Rising + declining queries AND topics in one call
mcp-cli call google-trends get_trending_analysis '{"timeframe":"today 1-m","geo":"FR"}'
```

### Tool Reference

| Tool | Returns | Best For |
|------|---------|----------|
| `full_keyword_research` | Everything: time series + related queries + topics + regions + suggestions | **First call** — minimize API usage |
| `compare_keywords` | Time series + summary stats per keyword | Content prioritization |
| `get_related_queries` | Full top + rising tables with value scores | Article H2s, FAQ questions, cluster topics |
| `get_interest_by_region` | All regions sorted by interest + geo codes | Geo-targeting content |
| `get_related_topics` | Full top + rising tables | Content adjacency discovery |
| `get_trending_searches` | Today's trending terms by country | Reactive/breaking content |
| `get_realtime_trending` | Real-time RSS trends with images | Breaking news, trending topics |
| `get_suggestions` | Autocomplete long-tails | Long-tail keyword expansion |
| `get_trending_analysis` | Rising + declining queries & topics | Trend direction analysis |

### ⚠️ Important Notes

- **No API key needed** — pytrends-modern scrapes Google Trends directly
- **`full_keyword_research`** = ONE call = 4-5 API requests batched internally → USE THIS FIRST
- **Rate limit = 429** → wait 60s before retrying, max 10-15 queries per session
- **Geo codes**: empty = worldwide, or use ISO codes: `FR`, `CH`, `DE`, `US`, `GB`
- **Timeframes**: `today 12-m` (1 year), `today 3-m` (3 months), `now 7-d` (7 days), `now 1-d` (24h), or custom `YYYY-MM-DD YYYY-MM-DD`

---

## SSH Server (use mcp-cli)

**Info:** `mcp-cli -h`

```bash
# Main SaaS server (Bene2Luxe, ZovaBoost, Netfluxe, OGBoujee)
mcp-cli call ssh-mpc-server-multisaasdeploy execute-command '{"cmdString":"ls -la"}'
```

---

## Voice Output

**TTS**: `elia-voxtral-speak` (Mistral, fast, French) → fallback: `elia-speak`

**⚠️ ALWAYS use the right tone:**
| Flag | Tone | When to use |
|------|------|-------------|
| `-j` | Happy | Good news, celebrations, positive updates |
| `-d` | Sad | Blockers, problems, urgent issues |
| `-a` | Angry | Urgent warnings, serious issues |
| `--play` | Play | Play audio after generation |
| `-x` | Fallback | Use elia-speak fallback |

```bash
elia-voxtral-speak "Great news!" -j           # happy
elia-voxtral-speak "Problem detected" -d      # sad
elia-voxtral-speak "URGENT!" -a               # angry
elia-voxtral-speak "Message" --play          # play audio
elia-speak -x "Message"                        # fallback
```

**⚠️ When sending a voice message to someone or a group:** ALWAYS use `elia-voxtral-speak` (or `elia-speak -x` as fallback) to generate the voice audio before sending via WhatsApp/Discord.

---

## ProtonMail CLI

**Info:** `~/.local/bin/protonmail -h`

```bash
# Location: ~/.local/bin/protonmail
~/.local/bin/protonmail list                    # Inbox
~/.local/bin/protonmail list -t sent           # Sent
~/.local/bin/protonmail list -t drafts         # Drafts
~/.local/bin/protonmail list -t spam          # Spam
~/.local/bin/protonmail list -t allmail        # All mail
```

---

## 🚀 ULW-Loop / Ralph-Loop (Autonomous Task Execution)

**Ralph-Loop = DEFAULT mode for cronjob runs (50 iterations max)**

### When to Use
- Task found during cronjob → execute it fully, don't just report
- Multiple tasks to do → run ralph-loop to process all of them
- Autonomous work mode → runs until `<promise>COMPLETE</promise>` or max iterations

### How to Launch (ALWAYS USE THIS FORMAT)

```
/ralph-loop
```

**⚠️ MANDATORY format for ALL runs:**
```bash
# Prefix with /ralph-loop + big-pickle model
oh-my-opencode run -a elia --model opencode/big-pickle "/ralph-loop YOUR TASK HERE" --attach
```

**⚠️ ALWAYS use `opencode/big-pickle` model** (free, stable, DEFAULT)

### What Happens
- Runs with ralph-loop (50 iterations max default)
- Spawns subagents for parallel execution
- Delegates to specialized agents (marketing, dev, sales, etc.)

**⚠️ CRITICAL: Why oh-my-opencode?**
- Regular `opencode run` does NOT resolve slash commands properly
- The LLM receives raw text instead of executing the command
- Use `oh-my-opencode run -a elia` for proper command execution

### Using Custom Subagents

**⚠️ ALWAYS provide context to subagents!**

When delegating to subagents, ALWAYS include:
- Relevant file paths (context, docs, reports)
- Business background needed
- Specific task details

```bash
# Start with elia, then delegate to subagents via task()
oh-my-opencode run -a elia "Message for subagent" --attach
# Then use task() to spawn specialized agents

# Available Subagents (via task(category="..."))
task(category="bene2luxe", ...)     # Luxury e-commerce
task(category="cobou-agency", ...) # B2B digital
task(category="zovaboost", ...)     # SMMPanel
task(category="setbon", ...)        # Marketing & Conversion
task(category="gilfoyle", ...)      # Backend dev
task(category="tiktok-youtube-auto", ...) # TikTok/YouTube auto
```

### Mandatory Context for Subagents

**ALWAYS give subagents the context they need:**

| Context File | When to Use |
|-------------|------------|
| `@context/business.md` | Tasks related to businesses, teams, operations |
| `@context/TOOLS.md` | Tasks needing tools, MCP, automation |
| `@memory/MEMORY.md` | Tasks needing long-term memory, blockers |
| `@docs/YYYY-MM-DD/*.md` | Tasks needing recent work/ reports |

**Example prompts WITH context:**
```bash
# GOOD - has context
task(category="bene2luxe", prompt="Create product listing for Chanel bag. 
Context: @context/business.md (team roles: Ali handles suppliers)
Recent: @docs/2026-04-20/morning_report.md
Tools: @context/TOOLS.md (image generation)")

# BAD - no context
task(category="bene2luxe", prompt="Create product listing")
```

**Context paths to reference:**
- Business context: `/Users/vakandi/EliaAI/context/business.md`
- Tools: `/Users/vakandi/EliaAI/context/TOOLS.md`
- Memory: `/Users/vakandi/EliaAI/memory/MEMORY.md`
- Today docs: `/Users/vakandi/EliaAI/docs/2026-04-20/`
- Jira: `https://bsbagency.atlassian.net` (project BEN, COBOUAGENC, etc.)

### High-Value Tasks to Execute
- Create accounts (Copify, Trendtrack, ad networks)
- Browse competitors (Wael's Edge folders: SHOP-5000$COPIFY, SHOP500-1000$COPIFY)
- Build React mockups for conversion ideas
- Research competitors and market opportunities
- Prepare sales documents and proposals
- ANYTHING that moves businesses forward

---

## 🚀 Starting New Work Sessions

```bash
# Quick start (uses big-pickle by default, ULW-loop enabled)
./start_agents.sh

# With extra context / task to execute
./start_agents.sh --extra-prompt="Create a Copify account and research ad earnings recovery"

# With proxy enabled (uses proxychains4)
./start_agents.sh --proxy --extra-prompt="Task description here"
```

**Options:**
| Flag | Purpose |
|------|---------|
| `--extra-prompt="..."` | Add task context to agent |
| `--proxy` | Run through proxychains4 |
| `--model=big-pickle` | Default (free, stable) |

---

## 🖼️ Image Generation (Bene2Luxe - Higgsfield.ai)

**⚠️ ALWAYS use this script for image generation** - Script: `/Users/vakandi/Documents/HiggsFieldGenerator/generate_photo_higgsfield.py`

The script automatically uses UNLIMITED mode (free, no credits).

### Quick Commands

```bash
cd /Users/vakandi/Documents/HiggsFieldGenerator

# Mascoot scenarios
python3 generate_photo_higgsfield.py mascoot vacation
python3 generate_photo_higgsfield.py mascoot party --items "champagne"
python3 generate_photo_higgsfield.py mascoot store --model gpt_image

# Human with product
python3 generate_photo_higgsfield.py human "Chanel bag"
python3 generate_photo_higgsfield.py human "Dior sneakers" --scenario wearing_sneakers

# With reference images (multi-photo support!)
python3 generate_photo_higgsfield.py human "Luxury Bag" --images "/path/to/photo1.png" "/path/to/photo2.png"
python3 generate_photo_higgsfield.py mascoot vacation --images "store.png" "mascot.png"

# Multiple models (queue mode)
python3 generate_photo_higgsfield.py human "Chanel bag" --models "gpt_image,soul_v2,flux_2_pro"
```

### CLI Options

| Flag | Description |
|------|-------------|
| `--model`, `-m` | Model (default: gpt_image) |
| `--models`, `-M` | Comma-separated models for batch |
| `--images`, `-i` | Reference images (paths) |
| `--output`, `-o` | Output file path |

### Available Models (Unlimited/Free)

| Model | Command |
|-------|---------|
| GPT Image | `gpt_image` |
| FLUX.2 Pro | `flux_2_pro` |
| Soul V2 | `soul_v2` |
| Nano Banana | `nano_banana` |
| Seedream 4.0 | `seedream_4_0` |

### Queue Management (Automatic)

The script monitors queue via eval:
- `get_queue_status()` - Returns queueCounter, processingCount, queueFull
- `wait_for_queue_slot()` - Waits for available slot
- `list_generation_items()` - Lists all generation items

### How It Works (Technical)

1. **Clear storage** - `localStorage.clear()`
2. **Enable unlimited** - `document.querySelector('[role="switch"]').click()`
3. **Upload images** - `agent-browser upload "[type=file]" "/path/to/image.png"`
4. **Set prompt** - Via localStorage injection + reload
5. **Click Generate** - Click the Unlimited button
6. **Monitor queue** - Via JavaScript eval

**Documentation**: `/Users/vakandi/Documents/HiggsFieldGenerator/docs/GENERATE_PHOTO_README.md`

---

## 🧠 MemPalace (AI Memory System)

**MemPalace** is an AI memory system that indexes your documentation with semantic search.

### ⚠️ CRITICAL: Docs and Brain are COPIES of each other

**NEVER search both at once** — they contain the SAME content in different formats:

| Folder | Format | Content |
|--------|--------|---------|
| `docs/` | Raw daily docs | Original files (chat logs, transcriptions, reports) |
| `brain/` | Obsidian wiki | Same content but with wiki-links between notes |

**They are mirrors of each other.** Don't combine searches — pick ONE:

- **docs/** = lire le contenu brut, les logs quotidiens
- **brain/** = lire les notes avec liens wiki (obsidian)

### Quick Commands

```bash
# Initialize (one-time per folder)
mempalace init /path/to/folder --yes

# Index content (create vector store)
mempalace mine /path/to/folder

# Search (pick ONE folder - they are duplicates!)
mempalace search /Users/vakandi/EliaAI/docs/ "query"
# OR
mempalace search /Users/vakandi/EliaAI/brain/ "query"
```

### Current Setup

**Two wings (DUPLICATES) in the same palace:**

| Wing | Path | Files | Rooms |
|------|------|-------|-------|
| `docs` | `/Users/vakandi/EliaAI/docs/` | 803 | diagram_elia_structure, general |
| `brain` | `/Users/vakandi/EliaAI/brain/` | 857 | linkers, documentation, frontend, raw, general |

**Both contain the same content** — choose one based on search needs:
- docs: raw files, daily logs, transcriptions
- brain: Obsidian vault with wiki-links connecting concepts

---

## 🎯 Task Delegation (during ulw-loop)

```typescript
// Development tasks
task(category="backend-dev", load_skills=["coding-agent"], prompt="...")

// Marketing tasks
task(category="marketing-social", load_skills=[], prompt="...")

// E-commerce (Bene2Luxe)
task(category="ecommerce-luxury", load_skills=["luxury-fashion-marketing-genius"], prompt="...")

// Multiple parallel agents
task(run_in_background=true, ...) // x3-5 in parallel
```

---

**Full details**: See PROMPT.md section "MCP Tools" and "Voice Transcription"

---

## 🔍 OpenCode Session Tools (Cron Job Context)

**CRITICAL**: These tools let Elia access previous cron job sessions for continuity and context.

### Why Use These?

When starting a new session, Elia should check what the previous cron runs did to:
- Avoid duplicating work
- Get context on pending tasks
- Understand what was already attempted
- See what failed and why

### Session Management Tools

```bash
# List recent sessions (last 20 by default)
session_list(limit=20, from_date="2026-03-29")

# Search sessions for specific topics/tasks
session_search(query="Bene2Luxe bug fix", limit=10)

# Read full session history
session_read(session_id="ses_abc123", include_todos=true)

# Get session metadata (date range, message count, agents used)
session_info(session_id="ses_abc123")
```

### Cron Job Session Pattern

**Every cron run creates a session**. Key patterns:

| Pattern | Session ID Prefix | Description |
|---------|------------------|--------------|
| `opencode_interactive_YYYYMMDD_HHMMSS.log` | `ses_2c...` | Main cron runs |
| Morning runs | `ses_2c7...` | 09:00-10:00 |
| Midday runs | `ses_2c5...` | 12:00-13:00 |
| Afternoon runs | `ses_2c6...` | 14:00-18:00 |
| Evening runs | `ses_2c4...` | 18:00-23:00 |

### Workflow: Check Previous Work Before Starting

```
1. session_list() → Find recent cron sessions
2. session_search(query="task or topic") → Find specific discussions
3. session_read(session_id) → Read what was done
4. Proceed with new work, avoiding duplication
```

### Example: Checking Yesterday's Work

```bash
# Find all sessions from yesterday
session_list(from_date="2026-03-29", limit=20)

# Search for specific task (e.g., "casquette" or "hat sizes")
session_search(query="casquette sizes bug", limit=5)

# Read a specific session to see what was attempted
session_read(session_id="ses_2c5e08b72ffe8LHrj9AAGc7dyI")
```

### Session Log Files

Sessions are also saved as log files:
```
/Users/vakandi/EliaAI/logs/
├── opencode_interactive_20260329_160001.log    # Cron run at 16:00
├── opencode_interactive_20260329_180001.log    # Cron run at 18:00
├── opencode_interactive_20260330_120001.log    # Today's run
└── ...
```

**Quick check via bash:**
```bash
ls -la /Users/vakandi/EliaAI/logs/opencode_interactive_2026*.log | tail -10
```

### Context Continuity Rule

**MANDATORY at start of each session:**
1. Check `session_list()` for recent cron runs
2. Check `session_search()` for relevant past discussions
3. Read key sessions if unclear about current state
4. Never repeat work that's already done
5. Note what failed before and why

This ensures Elia maintains context across cron runs and doesn't waste time re-doing work or repeating mistakes.
