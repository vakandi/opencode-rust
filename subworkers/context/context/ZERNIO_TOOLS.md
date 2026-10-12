# Zernio MCP Tools — Complete Reference

All posting tools, platform-specific content types, schemas, and usage examples.

---

## Table of Contents

1. [Supported Platforms & Content Types](#supported-platforms--content-types)
2. [Posting Tools](#posting-tools)
3. [Platform-Specific Data (platformSpecificData)](#platform-specific-data)
4. [TikTok Settings (tiktokSettings)](#tiktok-settings)
5. [Post Management Tools](#post-management-tools)
6. [Comment Management Tools](#comment-management-tools)
7. [Queue Scheduling Tools](#queue-scheduling-tools)
8. [Analytics Tools](#analytics-tools)
9. [Quick Reference: Every Platform x Content Type](#quick-reference)

---

## Supported Platforms & Content Types

| Platform | Platform ID | Content Types | Stories | Reels/Shorts | Threads | Carousels |
|----------|-------------|---------------|---------|--------------|---------|-----------|
| Twitter/X | `twitter` | tweet, thread | — | — | ✅ `threadItems` | — |
| Instagram | `instagram` | feed, story, reel, carousel | ✅ `contentType:"story"` | ✅ `contentType:"reel"` | — | ✅ (multi-image) |
| Facebook | `facebook` | feed, story, reel, carousel | ✅ `contentType:"story"` | ✅ `contentType:"reel"` | — | ✅ `carouselCards` |
| LinkedIn | `linkedin` | post, article | — | — | — | ✅ (up to 20 images) |
| TikTok | `tiktok` | video, photo | ❌ No stories | — | — | ✅ (photo posts) |
| YouTube | `youtube` | video, short | — | ✅ auto-detect | — | — |
| Pinterest | `pinterest` | pin, video pin, idea pin | — | — | — | — |
| Reddit | `reddit` | text, image, video, link, poll | — | — | — | — |
| Bluesky | `bluesky` | post, image, video | — | — | — | — |
| Threads | `threads` | post, carousel | — | — | ✅ `threadItems` | ✅ (up to 10 images) |
| Google Business | `googlebusiness` | update, offer, event, product | — | — | — | — |
| Snapchat | `snapchat` | story, saved_story, spotlight | ✅ `contentType:"story"` | ✅ spotlight | — | — |
| Telegram | `telegram` | message, media, album | — | — | — | ✅ (up to 10 media) |
| Discord | `discord` | message | — | — | — | — |
| WhatsApp | `whatsapp` | message, media | — | — | — | — |

---

## Posting Tools

### `zernio_schedule_post` — Multi-platform scheduled post

Schedule or publish to multiple platforms simultaneously with a single call.

**Required params:** `account_id`, `content`, `platforms[]`

**Optional params:** `scheduled_for`, `timezone`, `publish_now`, `media_urls`, `media_paths`, `media_type`, `queued_from_profile`, `queue_id`, `platformSpecificData`, `tiktokSettings`

```bash
# Multi-platform post (same content to 3 platforms)
mcp-cli call zernio zernio_schedule_post '{
  "account_id":"account-1",
  "content":"Big announcement! 🎉",
  "platforms":[
    {"platform":"twitter","accountId":"TWITTER_ACCOUNT_ID"},
    {"platform":"instagram","accountId":"IG_ACCOUNT_ID"},
    {"platform":"facebook","accountId":"FB_ACCOUNT_ID"}
  ],
  "publish_now":true
}'

# Scheduled post with media
mcp-cli call zernio zernio_schedule_post '{
  "account_id":"account-1",
  "content":"Coming soon...",
  "platforms":[{"platform":"instagram","accountId":"IG_ACCOUNT_ID"}],
  "scheduled_for":"2026-07-15T14:00:00+02:00",
  "media_paths":["/tmp/promo.mp4"]
}'
```

### `zernio_quick_post` — Single-platform quick post

Auto-resolves the connected social account from a profile. Simpler than `schedule_post` for single-platform publishing.

**Required params:** `account_id`, `content`, `platform`

**Optional params:** `profile_id`, `scheduled_for`, `timezone`, `publish_now`, `media_urls`, `media_paths`, `media_type`, `queued_from_profile`, `queue_id`, `platformSpecificData`, `tiktokSettings`

```bash
# Quick post to Instagram
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Hello world!",
  "platform":"instagram",
  "profile_id":"PROFILE_ID",
  "publish_now":true
}'

# Quick post with image
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Check this out",
  "platform":"twitter",
  "profile_id":"TWITTER_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/photo.jpg"]
}'
```

---

## Platform-Specific Data

Pass `platformSpecificData` inside `zernio_schedule_post` (top-level) or inside each platform entry when using the raw API. These override default behavior per-platform.

### Twitter/X

Twitter threads via `threadItems`. No stories/reels — just tweets.

```json
{
  "platformSpecificData": {
    "threadItems": [
      { "content": "Second tweet in the thread" },
      { "content": "Third tweet with image", "mediaItems": [{"type":"image","url":"https://..."}] },
      { "content": "Final tweet with a link" }
    ]
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `threadItems` | `array` | Additional tweets after the first (which uses `content`). Max ~25 tweets. |
| `threadItems[].content` | `string` | Text for that tweet |
| `threadItems[].mediaItems` | `array` | Optional media attachments per tweet |

**Usage — Twitter thread:**
```bash
mcp-cli call zernio zernio_schedule_post '{
  "account_id":"account-1",
  "content":"🧵 Thread about our latest feature (tweet 1/3)",
  "platforms":[{"platform":"twitter","accountId":"TWITTER_ACC"}],
  "publish_now":true,
  "platformSpecificData":{
    "threadItems":[
      {"content":"Here is the key insight (2/3)"},
      {"content":"So what does this mean? (3/3)"}
    ]
  }
}'
```

---

### Instagram

Instagram supports the richest set of content types: **feed posts**, **stories**, **reels**, **carousels**, and **trial reels**.

**⚠️ Story requirement:** Stories MUST use 9:16 aspect ratio media. If media is not 9:16, Instagram silently posts as feed instead of story. Text captions are not displayed on stories.

```json
{
  "platformSpecificData": {
    "contentType": "story",
    "firstComment": "First comment with hashtags!",
    "collaborators": ["username1"],
    "shareToFeed": true,
    "userTags": [{"username":"user","x":0.5,"y":0.5}],
    "trialParams": {"graduationStrategy": "SS_PERFORMANCE"},
    "audioName": "My Custom Audio",
    "thumbOffset": 5000
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `contentType` | `string` | `"story"` (24h ephemeral), `"reel"` (short video), or omit for feed/carousel |
| `firstComment` | `string` | Auto-posted first comment immediately after publishing |
| `collaborators` | `string[]` | Instagram usernames to invite as collaborators |
| `shareToFeed` | `boolean` | Share story/reel to feed (default: false for stories) |
| `userTags` | `array` | Tag users: `{username, x, y}` (0-1 normalized coordinates) |
| `trialParams` | `object` | Trial Reel settings (non-followers see first): `{graduationStrategy: "MANUAL" \| "SS_PERFORMANCE"}` |
| `audioName` | `string` | Custom label for original audio in Reels |
| `thumbOffset` | `number` | Thumbnail frame selection (milliseconds from video start) |

**Usage — Instagram Story (24h ephemeral):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Story caption (won\'t display on IG stories)",
  "platform":"instagram",
  "profile_id":"IG_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/story-image.jpg"],
  "platformSpecificData":{"contentType":"story"}
}'
```

**Usage — Instagram Reel:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"New reel! #fyp #viral",
  "platform":"instagram",
  "profile_id":"IG_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/reel.mp4"],
  "platformSpecificData":{"contentType":"reel","audioName":"Trending Sound"}
}'
```

**Usage — Instagram Trial Reel (shown to non-followers first):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Testing this reel idea",
  "platform":"instagram",
  "profile_id":"IG_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/test-reel.mp4"],
  "platformSpecificData":{
    "contentType":"reel",
    "trialParams":{"graduationStrategy":"SS_PERFORMANCE"}
  }
}'
```

**Usage — Instagram Carousel (multi-image feed post):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Swipe for the full story →",
  "platform":"instagram",
  "profile_id":"IG_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/slide1.jpg","/tmp/slide2.jpg","/tmp/slide3.jpg"],
  "platformSpecificData":{"firstComment":"Which slide is your favorite?"}
}'
```

**Instagram content type decision logic:**
- 1 image → feed post
- 1 video ≤90s, vertical 9:16 → Reel (default) — use `contentType:"reel"` to force
- 1 video >90s or horizontal → feed video
- Multiple images → carousel
- `contentType:"story"` → 24h ephemeral story (requires media)

---

### Facebook

Facebook supports **feed posts**, **stories**, **reels**, and **carousels** via `carouselCards`.

**⚠️ Story requirement:** Stories MUST use 9:16 aspect ratio media. Same as Instagram — non-9:16 media silently posts as feed.

```json
{
  "platformSpecificData": {
    "contentType": "story",
    "firstComment": "First comment!",
    "pageId": "123456789",
    "allowViewStory": true,
    "carouselCards": [
      {"name": "Product 1", "description": "Best seller", "link": "https://..."},
      {"name": "Product 2", "description": "New arrival", "link": "https://..."}
    ],
    "carouselLink": "https://example.com/shop"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `contentType` | `string` | `"story"` (24h ephemeral), `"reel"` (short video), or omit for feed |
| `firstComment` | `string` | Auto-posted first comment |
| `pageId` | `string` | Target Facebook Page ID (multi-page accounts) |
| `allowViewStory` | `boolean` | Allow viewing story after 24h |
| `carouselCards` | `array` | Multi-link carousel: `{name, description, link}` per card. All media must be images. |
| `carouselLink` | `string` | "See more" link on carousel end card (defaults to first card's link) |

**Usage — Facebook Story:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-3",
  "content":"FB story",
  "platform":"facebook",
  "profile_id":"FB_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/story.mp4"],
  "platformSpecificData":{"contentType":"story","allowViewStory":true}
}'
```

**Usage — Facebook Carousel:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-3",
  "content":"Check out our top products!",
  "platform":"facebook",
  "profile_id":"FB_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/prod1.jpg","/tmp/prod2.jpg","/tmp/prod3.jpg"],
  "platformSpecificData":{
    "carouselCards":[
      {"name":"Classic Tee","description":"$29.99","link":"https://shop.com/tee"},
      {"name":"Luxury Bag","description":"$199","link":"https://shop.com/bag"},
      {"name":"Gold Watch","description":"$499","link":"https://shop.com/watch"}
    ],
    "carouselLink":"https://shop.com"
  }
}'
```

---

### LinkedIn

LinkedIn supports **text posts**, **image posts** (up to 20 images), **video posts**, and **PDF document shares**.

```json
{
  "platformSpecificData": {
    "firstComment": "Drop your thoughts below!",
    "disableLinkPreview": false
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `firstComment` | `string` | Auto-posted first comment (great for adding hashtags without cluttering the post) |
| `disableLinkPreview` | `boolean` | Suppress link preview card |

**Usage — LinkedIn post with first comment for hashtags:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Excited to announce our new product launch!",
  "platform":"linkedin",
  "profile_id":"LINKEDIN_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/product-shot.jpg"],
  "platformSpecificData":{"firstComment":"#productlaunch #startup #saas"}
}'
```

**Usage — LinkedIn multi-image post:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Behind the scenes at our office",
  "platform":"linkedin",
  "profile_id":"LINKEDIN_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/bts1.jpg","/tmp/bts2.jpg","/tmp/bts3.jpg"]
}'
```

---

### TikTok

TikTok supports **video posts** and **photo/carousel posts**. NO stories. Rich privacy and interaction controls.

**⚠️ Required fields:** `contentPreviewConfirmed` and `expressConsentGiven` must be `true`.

```json
{
  "platformSpecificData": {
    "privacyLevel": "PUBLIC_TO_EVERYONE",
    "allowComment": true,
    "allowDuet": true,
    "allowStitch": true,
    "contentPreviewConfirmed": true,
    "expressConsentGiven": true,
    "draft": false,
    "commercialContentType": "none",
    "videoMadeWithAi": false,
    "videoCoverTimestampMs": 1000,
    "photoCoverIndex": 0,
    "autoAddMusic": false,
    "description": "Extended description for photo posts (max 4000 chars)"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `privacyLevel` | `string` | `"PUBLIC_TO_EVERYONE"`, `"FRIENDS"`, or `"SELF"` |
| `allowComment` | `boolean` | Allow comments |
| `allowDuet` | `boolean` | Allow duets (videos only) |
| `allowStitch` | `boolean` | Allow stitch (videos only) |
| `contentPreviewConfirmed` | `boolean` | **Required**: user confirmed content preview |
| `expressConsentGiven` | `boolean` | **Required**: express consent for posting |
| `draft` | `boolean` | Send to Creator Inbox instead of publishing |
| `commercialContentType` | `string` | `"none"`, `"brand_organic"`, `"brand_content"` |
| `brandPartnerPromote` | `boolean` | Promote a brand partner |
| `isBrandOrganicPost` | `boolean` | Brand organic post flag |
| `videoMadeWithAi` | `boolean` | AI-generated content disclosure |
| `videoCoverTimestampMs` | `number` | Video cover frame (ms from start) |
| `photoCoverIndex` | `number` | Cover index for photo carousels |
| `autoAddMusic` | `boolean` | Auto-add trending music |
| `description` | `string` | Extended description for photo posts (max 4000 chars) |

**Usage — TikTok video (public):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"#fyp #viral TikTok caption",
  "platform":"tiktok",
  "profile_id":"TIKTOK_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/tiktok-video.mp4"],
  "platformSpecificData":{
    "privacyLevel":"PUBLIC_TO_EVERYONE",
    "allowComment":true,
    "allowDuet":true,
    "allowStitch":true,
    "contentPreviewConfirmed":true,
    "expressConsentGiven":true
  }
}'
```

**Usage — TikTok video (friends only, no duet/stitch):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"Private TikTok video",
  "platform":"tiktok",
  "profile_id":"TIKTOK_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/video.mp4"],
  "platformSpecificData":{
    "privacyLevel":"FRIENDS",
    "allowComment":false,
    "allowDuet":false,
    "allowStitch":false,
    "contentPreviewConfirmed":true,
    "expressConsentGiven":true
  }
}'
```

**Usage — TikTok photo carousel:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"Photo dump from today",
  "platform":"tiktok",
  "profile_id":"TIKTOK_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/pic1.jpg","/tmp/pic2.jpg","/tmp/pic3.jpg"],
  "platformSpecificData":{
    "privacyLevel":"PUBLIC_TO_EVERYONE",
    "allowComment":true,
    "contentPreviewConfirmed":true,
    "expressConsentGiven":true,
    "description":"Extended description goes here for photo posts"
  }
}'
```

**Usage — TikTok draft (saves to Creator Inbox):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"Draft post",
  "platform":"tiktok",
  "profile_id":"TIKTOK_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/video.mp4"],
  "platformSpecificData":{
    "privacyLevel":"PUBLIC_TO_EVERYONE",
    "allowComment":true,
    "allowDuet":true,
    "allowStitch":true,
    "contentPreviewConfirmed":true,
    "expressConsentGiven":true,
    "draft":true
  }
}'
```

---

### YouTube

YouTube supports **regular videos** and **Shorts**. Shorts are auto-detected (vertical 9:16, ≤3 min).

```json
{
  "platformSpecificData": {
    "title": "Video Title",
    "visibility": "public",
    "firstComment": "Check out my other videos!",
    "containsSyntheticMedia": false
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `title` | `string` | Video title (separate from caption/content) |
| `visibility` | `string` | `"public"`, `"private"`, or `"unlisted"` |
| `firstComment` | `string` | Auto-posted first comment (max 10,000 chars) |
| `containsSyntheticMedia` | `boolean` | AI-generated content disclosure |

**Usage — YouTube Short (auto-detected from vertical video):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"YouTube Short #shorts",
  "platform":"youtube",
  "profile_id":"YOUTUBE_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/vertical-video.mp4"],
  "platformSpecificData":{"title":"My YouTube Short","visibility":"public"}
}'
```

**Usage — YouTube regular video:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"Full tutorial on our product",
  "platform":"youtube",
  "profile_id":"YOUTUBE_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/full-video.mp4"],
  "platformSpecificData":{
    "title":"Complete Product Tutorial 2026",
    "visibility":"public",
    "firstComment":"Thanks for watching! Like and subscribe!"
  }
}'
```

**YouTube Shorts auto-detection:**
- Vertical 9:16 + ≤3 min → classified as Short
- Horizontal 16:9 or >3 min → classified as regular video
- No manual flag needed — just upload the right aspect ratio

---

### Pinterest

Pinterest supports **pins**, **video pins**, and **idea pins** (multi-page stories).

```json
{
  "platformSpecificData": {
    "title": "Pin Title",
    "boardId": "board_123",
    "link": "https://example.com",
    "coverImageUrl": "https://example.com/cover.jpg",
    "coverImageKeyFrameTime": 5
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `title` | `string` | Pin title (max 100 chars, defaults to first line of content) |
| `boardId` | `string` | Target board ID (uses first available if omitted) |
| `link` | `string` | Click-through destination URL |
| `coverImageUrl` | `string` | Custom cover image for video pins |
| `coverImageKeyFrameTime` | `number` | Key frame time in seconds for video cover |

**Usage — Pinterest image pin:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Beautiful sunset at the beach",
  "platform":"pinterest",
  "profile_id":"PINTEREST_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/pin-image.jpg"],
  "platformSpecificData":{
    "title":"Beautiful Sunset Photos",
    "link":"https://example.com/blog/sunset-photos"
  }
}'
```

**Usage — Pinterest video pin:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Quick recipe tutorial",
  "platform":"pinterest",
  "profile_id":"PINTEREST_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/recipe-video.mp4"],
  "platformSpecificData":{
    "title":"Easy Pasta Recipe in 5 Minutes",
    "link":"https://example.com/recipes/pasta",
    "coverImageKeyFrameTime":3
  }
}'
```

---

### Reddit

Reddit supports **text posts**, **image posts**, **video posts**, **link posts**, and **polls**.

```bash
# Text post to a subreddit
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"What do you all think about the new update?",
  "platform":"reddit",
  "profile_id":"REDDIT_PROFILE_ID",
  "publish_now":true
}'
```

**Reddit posting notes:**
- Use `content` for post title + body (platform auto-splits)
- Target specific subreddits via the connected account's subreddit list
- Polls are created via the API when poll options are provided in the post body

---

### Bluesky

Bluesky supports **text posts**, **image posts** (up to 4 images), and **video posts**.

```bash
# Bluesky post with image
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Beautiful day for a walk",
  "platform":"bluesky",
  "profile_id":"BLUESKY_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/sunset.jpg"]
}'
```

---

### Threads

Threads supports **text posts**, **image posts** (up to 10 images as carousel), **video posts**, and **Threads threads** (reply chains via `threadItems`).

```json
{
  "platformSpecificData": {
    "threadItems": [
      { "content": "Second post in the thread" },
      { "content": "Third post with image", "mediaItems": [{"type":"image","url":"https://..."}] }
    ]
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `threadItems` | `array` | Additional posts after the first (which uses `content`). Creates a reply chain. |
| `threadItems[].content` | `string` | Text for that post in the thread |
| `threadItems[].mediaItems` | `array` | Optional media attachments per post |

**Usage — Threads thread (multi-post chain):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"🧵 Thread: Here are 5 tips for better content (1/5)",
  "platform":"threads",
  "profile_id":"THREADS_PROFILE_ID",
  "publish_now":true,
  "platformSpecificData":{
    "threadItems":[
      {"content":"Tip 1: Always hook in the first 3 seconds (2/5)"},
      {"content":"Tip 2: Use captions — most people watch on mute (3/5)"},
      {"content":"Tip 3: Post consistently, not just when you feel like it (4/5)"},
      {"content":"Tip 4: Engage with comments within the first hour (5/5)"}
    ]
  }
}'
```

**Usage — Threads carousel (multi-image):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Swipe to see the transformation →",
  "platform":"threads",
  "profile_id":"THREADS_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/before.jpg","/tmp/after.jpg"]
}'
```

---

### Google Business

Google Business supports **update posts**, **offer posts**, **event posts**, and **product posts**.

```json
{
  "platformSpecificData": {
    "callToAction": {
      "type": "LEARN_MORE",
      "url": "https://example.com"
    }
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `callToAction.type` | `string` | `"BOOK"`, `"ORDER"`, `"SHOP"`, `"LEARN_MORE"`, `"SIGN_UP"`, `"CALL"` |
| `callToAction.url` | `string` | Destination URL for the CTA button |

**Usage — Google Business post with CTA:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"We just launched our new service! Check it out.",
  "platform":"googlebusiness",
  "profile_id":"GB_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/promo.jpg"],
  "platformSpecificData":{
    "callToAction":{"type":"LEARN_MORE","url":"https://example.com/new-service"}
  }
}'
```

---

### Snapchat

Snapchat supports **stories** (24h ephemeral), **saved stories** (permanent on Public Profile), and **spotlight** (entertainment feed).

```json
{
  "platformSpecificData": {
    "contentType": "story"
  }
}
```

| Content Type | Duration | Notes |
|--------------|----------|-------|
| `story` | 24 hours | Ephemeral. No text caption displayed. Requires media. |
| `saved_story` | Permanent | Stays on Public Profile. Title max 45 chars. |
| `spotlight` | Permanent | For entertainment feed. Description max 160 chars. Video recommended. |

**Usage — Snapchat Story (24h ephemeral):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Story caption",
  "platform":"snapchat",
  "profile_id":"SNAP_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/snap-story.jpg"],
  "platformSpecificData":{"contentType":"story"}
}'
```

**Usage — Snapchat Spotlight (permanent entertainment feed):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Check out this crazy trick!",
  "platform":"snapchat",
  "profile_id":"SNAP_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/spotlight-video.mp4"],
  "platformSpecificData":{"contentType":"spotlight"}
}'
```

**Usage — Snapchat Saved Story (permanent on Public Profile):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Our story",
  "platform":"snapchat",
  "profile_id":"SNAP_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/saved-story.mp4"],
  "platformSpecificData":{"contentType":"saved_story"}
}'
```

---

### Telegram

Telegram supports **text messages**, **media messages**, and **albums** (up to 10 media in one message).

```json
{
  "platformSpecificData": {
    "parseMode": "HTML",
    "disableWebPagePreview": false,
    "disableNotification": false,
    "protectContent": false
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `parseMode` | `string` | `"HTML"`, `"Markdown"`, or `"MarkdownV2"` |
| `disableWebPagePreview` | `boolean` | Suppress link previews |
| `disableNotification` | `boolean` | Send silently (no notification sound) |
| `protectContent` | `boolean` | Prevent forwarding/saving |

**Usage — Telegram message with HTML formatting:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"<b>Big news!</b> <a href=\\"https://example.com\\">Click here</a> to learn more.",
  "platform":"telegram",
  "profile_id":"TELEGRAM_PROFILE_ID",
  "publish_now":true,
  "platformSpecificData":{"parseMode":"HTML"}
}'
```

**Usage — Telegram silent message with image:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"New product drop!",
  "platform":"telegram",
  "profile_id":"TELEGRAM_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/product.jpg"],
  "platformSpecificData":{"disableNotification":true,"protectContent":true}
}'
```

---

### Discord

Discord sends **messages** to channels. No stories/reels/threads — just plain text and media messages.

```bash
# Discord message
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Hello from Zernio! 🎉",
  "platform":"discord",
  "profile_id":"DISCORD_PROFILE_ID",
  "publish_now":true
}'
```

---

### WhatsApp

WhatsApp sends **messages** and **media** to contacts/groups. No stories/reels — messaging only.

```bash
# WhatsApp message with media
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Check out our new product!",
  "platform":"whatsapp",
  "profile_id":"WHATSAPP_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/product.jpg"]
}'
```

---

## TikTok Settings

TikTok has a **dual-path** for settings: you can use the dedicated `tiktokSettings` param (simpler, camelCase) OR pass the same fields inside `platformSpecificData`. Both work — `tiktokSettings` is a shorthand.

**`tiktokSettings` (top-level param in MCP tools):**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `privacyLevel` | `string` | `PUBLIC_TO_EVERYONE` | `"PUBLIC_TO_EVERYONE"`, `"FRIENDS"`, `"SELF"` |
| `allowComment` | `boolean` | `true` | Allow comments |
| `allowDuet` | `boolean` | `true` | Allow duets (videos only) |
| `allowStitch` | `boolean` | `true` | Allow stitch (videos only) |
| `contentPreviewConfirmed` | `boolean` | — | **REQUIRED**: must be `true` |
| `expressConsentGiven` | `boolean` | — | **REQUIRED**: must be `true` |
| `draft` | `boolean` | `false` | Send to Creator Inbox instead of publishing |
| `commercialContentType` | `string` | `"none"` | `"none"`, `"brand_organic"`, `"brand_content"` |
| `brandPartnerPromote` | `boolean` | `false` | Post promotes a brand partner |
| `isBrandOrganicPost` | `boolean` | `false` | Brand organic post flag |
| `videoMadeWithAi` | `boolean` | `false` | AI-generated content disclosure |
| `videoCoverTimestampMs` | `number` | — | Video cover frame (ms from start) |
| `photoCoverIndex` | `number` | — | Cover index for photo carousels |
| `autoAddMusic` | `boolean` | `false` | Auto-add trending music |
| `description` | `string` | — | Extended description for photo posts (max 4000 chars) |

**Usage — TikTok video with tiktokSettings (recommended for simplicity):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"#fyp #viral TikTok caption",
  "platform":"tiktok",
  "profile_id":"TIKTOK_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/tiktok-video.mp4"],
  "tiktokSettings":{
    "privacyLevel":"PUBLIC_TO_EVERYONE",
    "allowComment":true,
    "allowDuet":true,
    "allowStitch":true,
    "contentPreviewConfirmed":true,
    "expressConsentGiven":true
  }
}'
```

**Usage — TikTok video with platformSpecificData (equivalent):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"#fyp #viral TikTok caption",
  "platform":"tiktok",
  "profile_id":"TIKTOK_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/tiktok-video.mp4"],
  "platformSpecificData":{
    "privacyLevel":"PUBLIC_TO_EVERYONE",
    "allowComment":true,
    "allowDuet":true,
    "allowStitch":true,
    "contentPreviewConfirmed":true,
    "expressConsentGiven":true
  }
}'
```

**Usage — TikTok draft (sends to Creator Inbox):**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"Draft TikTok",
  "platform":"tiktok",
  "profile_id":"TIKTOK_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/video.mp4"],
  "tiktokSettings":{
    "privacyLevel":"PUBLIC_TO_EVERYONE",
    "allowComment":true,
    "contentPreviewConfirmed":true,
    "expressConsentGiven":true,
    "draft":true
  }
}'
```

**Usage — TikTok photo carousel with extended description:**
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"Photo dump from today",
  "platform":"tiktok",
  "profile_id":"TIKTOK_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/pic1.jpg","/tmp/pic2.jpg","/tmp/pic3.jpg"],
  "tiktokSettings":{
    "privacyLevel":"PUBLIC_TO_EVERYONE",
    "allowComment":true,
    "contentPreviewConfirmed":true,
    "expressConsentGiven":true,
    "description":"Extended description goes here for photo posts (max 4000 chars)"
  }
}'
```

---

## Post Management Tools

| Tool | Description |
|------|-------------|
| `zernio_list_scheduled_posts` | List scheduled posts with plaintext + JSON output, platform filter |
| `zernio_list_posts` | List posts with status/page/limit/account filters |
| `zernio_get_post` | Get a specific post by ID |
| `zernio_delete_post` | Delete a draft or scheduled post |
| `zernio_retry_post` | Retry a failed post |
| `zernio_unpublish_post` | Remove published post from platform (not IG/TikTok/Snap) |
| `zernio_update_post` | Update draft/scheduled/failed post (content, media, schedule) |
| `zernio_edit_published_post` | Edit published Twitter/X post (Premium, 1h window, 5 edits max) |
| `zernio_update_post_metadata` | Update YouTube metadata (title, description, tags) |

---

## Comment Management Tools

| Tool | Description |
|------|-------------|
| `zernio_list_commented_posts` | List posts with comment counts from inbox |
| `zernio_get_post_comments` | Get comments for a specific post |
| `zernio_reply_to_comment` | Reply to a post or specific comment thread |
| `zernio_delete_comment` | Delete a comment |
| `zernio_like_comment` | Like/upvote a comment |
| `zernio_hide_comment` | Hide a comment (FB/IG/Threads/X) |
| `zernio_unhide_comment` | Unhide a hidden comment |
| `zernio_private_reply_to_comment` | Send DM to comment author (IG/FB only, 7-day window) |
| `zernio_get_all_unreplied_comments` | Scan ALL accounts → return ALL comments with `needsReply` detection |

---

## Inbox & DM Tools (9)

Tools for managing DM conversations and comment-to-DM automations across Instagram, Facebook, WhatsApp, and X/Twitter.

### Comment-to-DM Automations

Automatically send DMs to users when they comment on your posts matching specific keywords. Supports Instagram and Facebook.

| Tool | Description |
|------|-------------|
| `zernio_create_comment_automation` | Create keyword-triggered DM automation (IG/FB) |
| `zernio_list_comment_automations` | List all automations for an account |
| `zernio_delete_comment_automation` | Delete an automation |

```bash
# Create a DM automation: when someone comments "price" send them a pricing DM
mcp-cli call zernio zernio_create_comment_automation '{
  "account_id":"account-1",
  "name":"Pricing inquiry auto-reply",
  "keywords":["price","how much","pricing","cost"],
  "matchMode":"contains",
  "dmMessage":"Hi! Thanks for your interest. Our pricing starts at $9/mo. Check out mirorpay.com for full details!",
  "commentReply":"Thanks for asking! I just sent you a DM with the details 😊"
}'

# List all automations
mcp-cli call zernio zernio_list_comment_automations '{"account_id":"account-1"}'

# Delete an automation
mcp-cli call zernio zernio_delete_comment_automation '{
  "account_id":"account-1",
  "automation_id":"AUTOMATION_ID"
}'
```

**`zernio_create_comment_automation` params:**

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `account_id` | string | ✅ | Account ID from `zernio_get_accounts` |
| `name` | string | ✅ | Automation name |
| `keywords` | string[] | ✅ | Trigger keywords |
| `matchMode` | string | ❌ | `"exact"` or `"contains"` (default) |
| `dmMessage` | string | ✅ | DM to auto-send |
| `commentReply` | string | ❌ | Optional comment reply before DM |
| `trigger` | string | ❌ | `"comment"` or `"story_reply"` |
| `platformPostId` | string | ❌ | Limit to one post |
| `dmMessageVariations` | string[] | ❌ | Random DM variants |
| `commentReplyVariations` | string[] | ❌ | Random comment reply variants |
| `linkTracking` | boolean | ❌ | Track DM link clicks |
| `clickTag` | string | ❌ | UTM tag for link tracking |

### DM Inbox

Manage direct message conversations across Instagram, Facebook, WhatsApp, and X/Twitter.

| Tool | Description |
|------|-------------|
| `zernio_list_inbox_conversations` | List DM conversations (filter by platform/status) |
| `zernio_get_inbox_conversation` | Get full message history for a conversation |
| `zernio_send_inbox_message` | Send a message to an existing conversation |
| `zernio_close_conversation` | Close/archive a conversation |
| `zernio_list_inbox_folders` | List inbox folders for unified inbox |
| `zernio_move_conversation` | Move conversation to a folder |

```bash
# List all Instagram DM conversations
mcp-cli call zernio zernio_list_inbox_conversations '{"account_id":"account-1","platform":"instagram"}'

# List only open conversations
mcp-cli call zernio zernio_list_inbox_conversations '{"account_id":"account-1","status":"open"}'

# Get full conversation history
mcp-cli call zernio zernio_get_inbox_conversation '{
  "account_id":"account-1",
  "conversation_id":"CONVERSATION_ID"
}'

# Reply to a conversation
mcp-cli call zernio zernio_send_inbox_message '{
  "account_id":"account-1",
  "conversation_id":"CONVERSATION_ID",
  "message":"Thanks for reaching out! How can I help you?"
}'

# Send DM with quick replies (Instagram/Facebook)
mcp-cli call zernio zernio_send_inbox_message '{
  "account_id":"account-1",
  "conversation_id":"CONVERSATION_ID",
  "message":"What would you like to know?",
  "quick_replies":[
    {"content_type":"text","title":"Pricing","payload":"pricing"},
    {"content_type":"text","title":"Features","payload":"features"},
    {"content_type":"text","title":"Support","payload":"support"}
  ]
}'

# Send outside 24h window (uses HUMAN_AGENT tag for 7-day window)
mcp-cli call zernio zernio_send_inbox_message '{
  "account_id":"account-1",
  "conversation_id":"CONVERSATION_ID",
  "message":"Following up on our conversation!",
  "message_tag":"HUMAN_AGENT"
}'

# Close a conversation
mcp-cli call zernio zernio_close_conversation '{
  "account_id":"account-1",
  "conversation_id":"CONVERSATION_ID"
}'

# List folders
mcp-cli call zernio zernio_list_inbox_folders '{"account_id":"account-1"}'

# Move conversation to a folder
mcp-cli call zernio zernio_move_conversation '{
  "account_id":"account-1",
  "conversation_id":"CONVERSATION_ID",
  "folder_id":"FOLDER_ID"
}'
```

**`zernio_send_inbox_message` params:**

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `account_id` | string | ✅ | Account ID |
| `conversation_id` | string | ✅ | Conversation to reply to |
| `message` | string | ✅ | Message text |
| `media_url` | string | ❌ | Attach image/video |
| `quick_replies` | array | ❌ | Quick reply buttons (IG/FB) |
| `buttons` | array | ❌ | Persistent menu buttons (FB) |
| `message_tag` | string | ❌ | `HUMAN_AGENT`, `CONFIRMED_EVENT_UPDATE`, `POST_PURCHASE_UPDATE`, `ACCOUNT_UPDATE` — allows messaging outside 24h window |
| `messaging_type` | string | ❌ | `RESPONSE`, `UPDATE`, `MESSAGE_TAG` |

**DM inbox best practices:**
- Use `message_tag: "HUMAN_AGENT"` to extend the 24h window to 7 days on IG/FB
- Filter by `platform` and `status` to keep the inbox manageable
- Use comment automations to auto-qualify leads before engaging in DMs
- Close conversations after resolution to keep the inbox clean

---

| Tool | Description |
|------|-------------|
| `zernio_list_queue_slots` | List queue schedules for a profile |
| `zernio_create_queue_slot` | Create a queue with time slots |
| `zernio_delete_queue_slot` | Delete a queue schedule |
| `zernio_preview_queue_slots` | Preview upcoming N slots |
| `zernio_next_queue_slot` | Get next available slot (preview) |

Queue posting: pass `queued_from_profile` (profileId) and optional `queue_id` to `zernio_schedule_post` or `zernio_quick_post` instead of `scheduled_for`.

---

## Analytics Tools

| Tool | Description |
|------|-------------|
| `zernio_get_post_analytics` | Single post analytics or overview |
| `zernio_get_analytics_timeline` | Daily timeline for a post |
| `zernio_get_daily_metrics` | Daily aggregated metrics (Analytics add-on) |
| `zernio_get_best_time_to_post` | Recommended best posting times |
| `zernio_get_follower_stats` | Follower counts across all accounts |

---

## How To Publish: Stories, Reels & Shorts

### Instagram Story (24h ephemeral)
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"Behind the scenes today 🎬",
  "platform":"instagram",
  "profile_id":"IG_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/story.jpg"],
  "platformSpecificData":{"contentType":"story","shareToFeed":false}
}'
```
- `contentType:"story"` = 24h ephemeral
- `shareToFeed:false` = don't show on profile grid (default for stories)
- **⚠️ CRITICAL: Media MUST be 9:16 aspect ratio** — otherwise Instagram silently posts as feed instead of story
- Requires media (image or video), no text-only stories
- Stories don't display text captions

### Instagram Reel (short video)
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-1",
  "content":"#fyp #viral New feature drop!",
  "platform":"instagram",
  "profile_id":"IG_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/reel.mp4"],
  "platformSpecificData":{"contentType":"reel","audioName":"Trending Sound"}
}'
```
- `contentType:"reel"` forces Reel (otherwise auto-detected for vertical ≤90s)
- `audioName` = custom label for original audio

### TikTok Story
**❌ TikTok does NOT support stories.** TikTok has:
- **Video posts** (standard)
- **Photo/carousel posts**
- Use `contentType: "video"` via media upload

### TikTok Video Post
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"#fyp #viral Check this out!",
  "platform":"tiktok",
  "profile_id":"TIKTOK_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/video.mp4"],
  "platformSpecificData":{
    "privacyLevel":"PUBLIC_TO_EVERYONE",
    "allowComment":true,
    "allowDuet":true,
    "allowStitch":true,
    "contentPreviewConfirmed":true,
    "expressConsentGiven":true
  }
}'
```
- `contentPreviewConfirmed:true` + `expressConsentGiven:true` are **required**
- No stories, no reels — just video/photo posts

### Facebook Story (24h ephemeral)
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-3",
  "content":"FB story",
  "platform":"facebook",
  "profile_id":"FB_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/story.mp4"],
  "platformSpecificData":{"contentType":"story","allowViewStory":true}
}'
```
- `contentType:"story"` = 24h ephemeral
- `allowViewStory:true` = allow viewing after 24h (optional)
- **⚠️ CRITICAL: Media MUST be 9:16 aspect ratio** — same as Instagram
- Requires media (image or video)

### Facebook Reel
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-3",
  "content":"Check out our new feature!",
  "platform":"facebook",
  "profile_id":"FB_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/reel.mp4"],
  "platformSpecificData":{"contentType":"reel"}
}'
```
- `contentType:"reel"` forces Reel
- Vertical video recommended

### YouTube Short
```bash
mcp-cli call zernio zernio_quick_post '{
  "account_id":"account-2",
  "content":"YouTube Short #shorts",
  "platform":"youtube",
  "profile_id":"YOUTUBE_PROFILE_ID",
  "publish_now":true,
  "media_paths":["/tmp/vertical-video.mp4"],
  "platformSpecificData":{"title":"My YouTube Short","visibility":"public"}
}'
```
- Auto-detected if vertical 9:16 + ≤3 min
- No manual flag needed — just upload the right aspect ratio
- `title` is separate from `content` (caption)

---

## Quick Reference

### Content Type Decision Table

| Want to post... | Platform | How |
|-----------------|----------|-----|
| Instagram Story (24h) | Instagram | `platformSpecificData: {"contentType":"story"}` + **9:16 media required** |
| Instagram Reel | Instagram | `platformSpecificData: {"contentType":"reel"}` + video |
| Instagram Carousel | Instagram | Multiple `media_paths` (images) |
| Instagram Trial Reel | Instagram | `platformSpecificData: {"contentType":"reel","trialParams":{"graduationStrategy":"SS_PERFORMANCE"}}` |
| Facebook Story (24h) | Facebook | `platformSpecificData: {"contentType":"story"}` + **9:16 media required** |
| Facebook Reel | Facebook | `platformSpecificData: {"contentType":"reel"}` + video |
| Facebook Carousel | Facebook | `platformSpecificData: {"carouselCards":[...]}` + images |
| Twitter Thread | Twitter/X | `platformSpecificData: {"threadItems":[...]}` |
| YouTube Short | YouTube | Upload vertical 9:16 video ≤3 min (auto-detected) |
| YouTube Video | YouTube | Upload horizontal/longer video + `platformSpecificData: {"title":"..."}` |
| TikTok Video | TikTok | `platformSpecificData: {"contentPreviewConfirmed":true,"expressConsentGiven":true,...}` + video |
| TikTok Photo Carousel | TikTok | Multiple `media_paths` (images) + TikTok settings |
| TikTok Draft | TikTok | `"draft":true` in platformSpecificData |
| Pinterest Pin | Pinterest | `platformSpecificData: {"title":"...","boardId":"..."}` + image |
| Pinterest Video Pin | Pinterest | `platformSpecificData: {"title":"..."}` + video |
| Snapchat Story (24h) | Snapchat | `platformSpecificData: {"contentType":"story"}` + media |
| Snapchat Saved Story | Snapchat | `platformSpecificData: {"contentType":"saved_story"}` + media |
| Snapchat Spotlight | Snapchat | `platformSpecificData: {"contentType":"spotlight"}` + video |
| Threads Thread | Threads | `platformSpecificData: {"threadItems":[...]}` |
| Threads Carousel | Threads | Multiple `media_paths` (up to 10 images) |
| Google Business + CTA | Google Business | `platformSpecificData: {"callToAction":{"type":"LEARN_MORE","url":"..."}}` |
| LinkedIn + Hashtags | LinkedIn | `platformSpecificData: {"firstComment":"#hashtags"}` |
| Telegram Formatted | Telegram | `platformSpecificData: {"parseMode":"HTML"}` + HTML in content |
| Telegram Silent | Telegram | `platformSpecificData: {"disableNotification":true}` |

### Media Requirements by Platform

| Platform | Image Formats | Video Formats | Max Images | Max Video Length |
|----------|---------------|---------------|------------|------------------|
| Instagram | jpg, png | mp4, mov | 10 (carousel) | 90s (reel) |
| Facebook | jpg, png, gif | mp4, mov | 10 | Varies |
| Twitter/X | jpg, png, gif, webp | mp4 | 4 | 140s |
| LinkedIn | jpg, png | mp4 | 20 | 10 min |
| TikTok | jpg, png | mp4, mov | 12 | 10 min |
| YouTube | jpg (thumbnail) | mp4, mov, webm | — | 12h (video) |
| Pinterest | jpg, png | mp4 | 1 (per pin) | 15 min |
| Reddit | jpg, png, gif | mp4 | 20 | 15 min |
| Bluesky | jpg, png, gif | mp4 | 4 | 60s |
| Threads | jpg, png | mp4 | 10 | 5 min |
| Snapchat | jpg, png | mp4 | 1 | 60s |
| Telegram | jpg, png, gif, webp | mp4 | 10 (album) | 50 MB |
| Discord | jpg, png, gif | mp4 | 10 | — |
| WhatsApp | jpg, png | mp4 | — | 16 MB |

### Story Lifecycle

| Platform | Story Type | Duration | Saved to Profile? | Media Requirement |
|----------|-----------|----------|-------------------|-------------------|
| Instagram | Story | 24 hours | No (unless Highlight) | **9:16 aspect ratio required** |
| Facebook | Story | 24 hours | No (unless `allowViewStory: true`) | **9:16 aspect ratio required** |
| Snapchat | Story | 24 hours | No | Any aspect ratio |
| Snapchat | Saved Story | Permanent | Yes (Public Profile) | Any aspect ratio |
| Snapchat | Spotlight | Permanent | Yes | Any aspect ratio |
| Threads | Story | 24 hours | No | Any aspect ratio |

### First Comment Feature

Several platforms support auto-posting a first comment immediately after publishing. Great for hashtags without cluttering the main post.

| Platform | Field | Notes |
|----------|-------|-------|
| Instagram | `platformSpecificData.firstComment` | Auto-posts comment |
| Facebook | `platformSpecificData.firstComment` | Auto-posts comment |
| LinkedIn | `platformSpecificData.firstComment` | Auto-posts comment |
| YouTube | `platformSpecificData.firstComment` | Max 10,000 chars |
| TikTok | — | No first comment support |
| Twitter/X | — | Use threadItems instead |

---

## ⚠️ Common Issues & Troubleshooting

### Instagram Story not appearing (but post succeeds)
**Root cause:** Media is not 9:16 aspect ratio. Instagram silently posts as feed instead of story.

**Fix:** Ensure media is exactly 9:16 (1080x1920px). Use `ffprobe` to check:
```bash
ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0 /tmp/story.jpg
```

**Solution:** Crop/resize to 9:16 before uploading:
```bash
ffmpeg -i /tmp/photo.jpg -vf "crop=ih*9/16:ih" /tmp/story-9-16.jpg
```

### Other platforms
- Stories require media (no text-only)
- Text captions are not displayed on IG/FB stories
- Link stickers not available via API (IG limitation)
