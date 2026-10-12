# HeyGen Content MCP — Full Tool Reference

> **Server:** `heygen-content-mcp`
> **Architecture:** 100% local — no HeyGen API key needed
> **Engines:** HyperFrames Whisper (transcribe), Kokoro-82M (TTS), Playwright+FFmpeg (render), FFmpeg (clip/cut/overlay), OpenRouter (AI analysis)

---

## Table of Contents

1. [Overview](#overview)
2. [Tool Reference](#tool-reference)
3. [Workflow: Edit a Video from Podcast/Competitor Content](#workflow)
4. [Examples](#examples)
5. [Tips & Best Practices](#tips)

---

## Overview <a name="overview"></a>

The HeyGen Content MCP is a local-first video editing pipeline. All processing happens on your machine — no API keys required (except `OPENROUTER_API_KEY` for AI transcript analysis).

**Core capabilities:**
- **Transcribe** any video/audio to word-level timestamps (Whisper)
- **AI analyze** transcripts to find the best moments for clips (OpenRouter)
- **Cut clips** at precise timestamps (FFmpeg)
- **Create vertical 9:16 clips** for TikTok/Reels/Shorts (FFmpeg)
- **Add text overlays**, logos, and watermarks with timestamp control (Pillow+FFmpeg)
- **Split-screen video** — two videos side-by-side or stacked for competitor comparisons (FFmpeg hstack/vstack)
- **Photo editing** — text, logo, collage, watermark, split screen on images (Pillow)
- **Generate TTS** voiceovers (Kokoro-82M)
- **Render HTML** compositions to video (Playwright+FFmpeg)

---

## Tool Reference <a name="tool-reference"></a>

### 🎙️ Transcription

#### `transcribe`
Transcribe audio/video to word-level timestamps using Whisper. Supports any language.

```bash
mcp-cli call heygen-content-mcp transcribe '{
  "file_path": "/path/to/video.mp4",
  "model": "medium.en",
  "language": "en"
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file_path` | string | ✅ | Path to audio or video file |
| `model` | string | ❌ | Whisper model: `tiny.en`, `base.en`, `small.en`, `medium.en`, `large-v3` (default: `base.en`) |
| `language` | string | ❌ | Language code to filter (e.g. `en`, `fr`, `es`). Empty = auto-detect. |

**Output:** JSON with word-level timestamps (start, end, word for each token)

---

### 🔊 Text to Speech

#### `tts`
Generate speech audio from text using Kokoro-82M. No API key needed.

```bash
mcp-cli call heygen-content-mcp tts '{
  "text": "Welcome to our channel!",
  "voice": "af_heart",
  "speed": 1.0,
  "output_filename": "intro.wav"
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `text` | string | ✅ | Text to speak (or path to .txt file) |
| `voice` | string | ❌ | Voice ID (default: `af_heart`) |
| `speed` | number | ❌ | Speed multiplier 0.1-3.0 (default: 1.0) |
| `output_filename` | string | ❌ | Output filename (default: `speech_<timestamp>.wav`) |
| `lang` | string | ❌ | Phonemizer language override (auto-detected from voice prefix) |

**Available voices:** `af_heart`, `af_bella`, `af_nicole`, `af_sarah`, `af_sky`, `am_adam`, `am_michael`, `bf_emma`, `bf_isabella`, `bm_george`, `bm_lewis`

#### `list_tts_voices`
List all available Kokoro-82M TTS voices.

```bash
mcp-cli call heygen-content-mcp list_tts_voices '{}'
```

---

### 🎬 Video Rendering

#### `render`
Render an HTML composition to video using Playwright+FFmpeg.

```bash
mcp-cli call heygen-content-mcp render '{
  "html_path": "/path/to/composition/",
  "output_filename": "render.mp4",
  "fps": 30,
  "quality": "standard"
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `html_path` | string | ✅ | Path to HTML composition file or project directory |
| `output_filename` | string | ❌ | Output video filename |
| `fps` | integer | ❌ | Frame rate: 24, 30, 60 (default: 30) |
| `quality` | string | ❌ | Quality: `draft`, `standard`, `high` (default: `standard`) |

#### `lint`
Lint an HTML composition for common mistakes before rendering.

```bash
mcp-cli call heygen-content-mcp lint '{"html_path": "/path/to/composition/"}'
```

#### `validate`
Runtime-validate an HTML composition (checks JS errors, missing assets).

```bash
mcp-cli call heygen-content-mcp validate '{"html_path": "/path/to/composition/"}'
```

---

### 🤖 AI Transcript Analysis

#### `analyze_transcript`
Use AI to analyze a transcript and find the best moments for short clips. Core tool for podcast→clips workflow.

```bash
mcp-cli call heygen-content-mcp analyze_transcript '{
  "transcript": "transcript.json",
  "max_clips": 5,
  "instruction": "Extract the 5 most engaging moments for social media",
  "min_duration": 15,
  "max_duration": 120,
  "model": "anthropic/claude-3.5-sonnet"
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `transcript` | string | ✅ | Transcript text (or path to transcript.json) |
| `max_clips` | integer | ❌ | Max clips to find (default: 5) |
| `instruction` | string | ❌ | Editorial guidance: topics, speaker focus, tone |
| `min_duration` | number | ❌ | Min clip duration in seconds (default: 15) |
| `max_duration` | number | ❌ | Max clip duration in seconds (default: 120) |
| `model` | string | ❌ | OpenRouter model (default: `anthropic/claude-3.5-sonnet`) |

**Output:** JSON with `clips` array containing `start`, `end`, `label`, `reason`, `hook` for each clip

---

### ✂️ Clip Operations

#### `cut_clip`
Cut a single clip from a video at specific timestamps.

```bash
mcp-cli call heygen-content-mcp cut_clip '{
  "video_path": "/path/to/video.mp4",
  "start": 120.5,
  "end": 180.2,
  "output_filename": "clip_funny_moment.mp4",
  "reencode": false
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `video_path` | string | ✅ | Path to source video |
| `start` | number | ✅ | Start time in seconds |
| `end` | number | ✅ | End time in seconds |
| `output_filename` | string | ❌ | Output filename (default: `clip_<timestamp>.mp4`) |
| `reencode` | boolean | ❌ | Re-encode for frame-accurate cuts (slower but precise) |

#### `cut_multiple_clips`
Cut multiple clips from a video in one call.

```bash
mcp-cli call heygen-content-mcp cut_multiple_clips '{
  "video_path": "/path/to/video.mp4",
  "windows": [
    {"start": 120, "end": 180, "label": "funny_moment"},
    {"start": 300, "end": 360, "label": "insight"},
    {"start": 450, "end": 510, "label": "call_to_action"}
  ],
  "reencode": false
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `video_path` | string | ✅ | Path to source video |
| `windows` | array | ✅ | List of windows to cut (max 20) |
| `reencode` | boolean | ❌ | Re-encode for frame-accurate cuts |

---

### 📝 Text Overlay & Captions

#### `add_text_overlay`
Burn text overlay onto a video. Uses Pillow-generated PNG + FFmpeg overlay (no drawtext dependency). Supports font, color, size, position, background box, outline, opacity, and timestamp control.

```bash
mcp-cli call heygen-content-mcp add_text_overlay '{
  "video_path": "/path/to/clip.mp4",
  "text": "YOUR BRAND",
  "position": "bottom",
  "font_size": 36,
  "font_color": "white",
  "bg_color": "#000000CC",
  "opacity": 1.0,
  "font_path": "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
  "outline_color": "black",
  "outline_width": 2,
  "start_time": 0,
  "end_time": 10
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `video_path` | string | ✅ | Path to source video |
| `text` | string | ✅ | Text to overlay |
| `position` | string | ❌ | Position: `center`, `top`, `bottom`, `top-left`, `top-right`, `bottom-left`, `bottom-right` (default: `center`) |
| `font_size` | integer | ❌ | Font size in pixels (default: 48) |
| `font_color` | string | ❌ | Font color: `white`, `black`, `red`, `#FF0000`, etc (default: `white`) |
| `bg_color` | string | ❌ | Background box color behind text. E.g. `black`, `#000000`, `#000000CC` (with alpha) |
| `opacity` | number | ❌ | Text opacity 0.0-1.0 (default: 1.0) |
| `font_path` | string | ❌ | Path to .ttf/.otf font file. Empty = system Arial |
| `outline_color` | string | ❌ | Text outline/stroke color (none for no outline) |
| `outline_width` | integer | ❌ | Outline thickness in pixels (default: 2) |
| `bg_padding` | integer | ❌ | Padding around text inside background box (default: 10) |
| `margin` | integer | ❌ | Margin from video edge in pixels (default: 50) |
| `start_time` | number | ❌ | When to show text in seconds from start (default: 0) |
| `end_time` | number | ❌ | When to hide text in seconds. Empty = show until end |
| `output_filename` | string | ❌ | Output filename |

#### `add_caption_track`
Burn a transcript/timestamped captions into video as hardcoded subtitles.

```bash
mcp-cli call heygen-content-mcp add_caption_track '{
  "video_path": "/path/to/clip.mp4",
  "transcript_path": "/path/to/transcript.json",
  "font_size": 24,
  "font_color": "white",
  "position": "bottom",
  "output_filename": "clip_with_captions.mp4"
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `video_path` | string | ✅ | Path to source video |
| `transcript_path` | string | ✅ | Path to transcript.json (word-level timestamps) |
| `font_size` | integer | ❌ | Font size (default: 24) |
| `font_color` | string | ❌ | Font color (default: `white`) |
| `position` | string | ❌ | Position: `bottom`, `center` (default: `bottom`) |
| `output_filename` | string | ❌ | Output filename |

---

### 🎬 Video Effects (Logo, Split Screen, Watermark)

All video effects use Pillow-generated PNG + FFmpeg overlay filter. No drawtext dependency. Every tool supports timestamp control (`start_time`/`end_time`) so you can show/hide effects at precise moments.

#### `add_logo_to_video`
Overlay a logo image on a video with position, scale, opacity, and timestamp control.

```bash
mcp-cli call heygen-content-mcp add_logo_to_video '{
  "video_path": "/path/to/video.mp4",
  "logo_path": "/path/to/logo.png",
  "position": "bottom-right",
  "scale": 0.15,
  "opacity": 0.9,
  "margin": 20,
  "start_time": 0,
  "end_time": 30
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `video_path` | string | ✅ | Path to source video |
| `logo_path` | string | ✅ | Path to logo image (PNG with transparency recommended) |
| `position` | string | ❌ | Position: `center`, `top`, `bottom`, `top-left`, `top-right`, `bottom-left`, `bottom-right` (default: `bottom-right`) |
| `scale` | number | ❌ | Logo scale relative to video width: 0.01-1.0 (default: 0.15 = 15%) |
| `opacity` | number | ❌ | Logo opacity 0.0-1.0 (default: 1.0) |
| `margin` | integer | ❌ | Margin from edges in pixels (default: 20) |
| `start_time` | number | ❌ | When to show logo in seconds (default: 0) |
| `end_time` | number | ❌ | When to hide logo in seconds. Empty = show until end |
| `output_filename` | string | ❌ | Output filename |

#### `split_screen_video`
Combine two videos side-by-side (horizontal) or stacked (vertical). Perfect for competitor comparisons — put a competitor video on top and your HeyGen illustration below.

```bash
# Vertical split: competitor video on TOP, your product video on BOTTOM
mcp-cli call heygen-content-mcp split_screen_video '{
  "video_a": "/path/to/competitor_video.mp4",
  "video_b": "/path/to/mirorpay_demo.mp4",
  "layout": "vertical",
  "start_a": 5,
  "start_b": 0,
  "duration_a": 15,
  "duration_b": 15,
  "audio_source": "a"
}'

# Horizontal split: side-by-side comparison
mcp-cli call heygen-content-mcp split_screen_video '{
  "video_a": "/path/to/video_a.mp4",
  "video_b": "/path/to/video_b.mp4",
  "layout": "horizontal",
  "start_a": 0,
  "start_b": 0,
  "duration_a": 10,
  "duration_b": 10,
  "audio_source": "a"
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `video_a` | string | ✅ | Path to first video (left or top) |
| `video_b` | string | ✅ | Path to second video (right or bottom) |
| `layout` | string | ❌ | `horizontal` (side-by-side) or `vertical` (stacked). Default: `horizontal` |
| `start_a` | number | ❌ | Start offset for video A in seconds (default: 0) |
| `start_b` | number | ❌ | Start offset for video B in seconds (default: 0) |
| `duration_a` | number | ❌ | Duration to take from video A in seconds (empty = full) |
| `duration_b` | number | ❌ | Duration to take from video B in seconds (empty = full) |
| `audio_source` | string | ❌ | Which audio to keep: `a` (video A), `b` (video B), `both`, `none` (default: `a`) |
| `output_filename` | string | ❌ | Output filename |

#### `add_video_watermark`
Add a semi-transparent text watermark to a video with timestamp control.

```bash
mcp-cli call heygen-content-mcp add_video_watermark '{
  "video_path": "/path/to/video.mp4",
  "text": "@mirorpay",
  "position": "bottom-right",
  "font_size": 24,
  "opacity": 0.4,
  "margin": 15,
  "start_time": 0,
  "end_time": 10
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `video_path` | string | ✅ | Path to source video |
| `text` | string | ✅ | Watermark text (e.g. brand name, @handle) |
| `position` | string | ❌ | Corner: `bottom-right`, `bottom-left`, `top-right`, `top-left` (default: `bottom-right`) |
| `font_size` | integer | ❌ | Font size in pixels (default: 24) |
| `font_color` | string | ❌ | Font color (default: `white`) |
| `opacity` | number | ❌ | Opacity 0.0-1.0 (default: 0.4) |
| `margin` | integer | ❌ | Margin from edges in pixels (default: 15) |
| `start_time` | number | ❌ | When to show watermark in seconds (default: 0) |
| `end_time` | number | ❌ | When to hide watermark in seconds. Empty = show until end |
| `output_filename` | string | ❌ | Output filename |

---

### 🎵 Audio Operations

#### `extract_audio`
Extract audio track from a video file.

```bash
mcp-cli call heygen-content-mcp extract_audio '{
  "video_path": "/path/to/video.mp4",
  "output_filename": "audio.wav",
  "format": "wav"
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `video_path` | string | ✅ | Path to source video |
| `output_filename` | string | ❌ | Output filename (default: `audio_<timestamp>.wav`) |
| `format` | string | ❌ | Output format: `wav`, `mp3`, `aac` (default: `wav`) |

#### `replace_audio`
Replace audio track in a video with a new audio file.

```bash
mcp-cli call heygen-content-mcp replace_audio '{
  "video_path": "/path/to/video.mp4",
  "audio_path": "/path/to/new_audio.wav",
  "output_filename": "video_with_new_audio.mp4"
}'
```

---

### 🔄 Format & Conversion

#### `convert_video`
Convert video format, resolution, or codec. Useful for preparing clips for social media.

```bash
mcp-cli call heygen-content-mcp convert_video '{
  "video_path": "/path/to/video.mp4",
  "output_format": "mp4",
  "width": 1080,
  "height": 1920,
  "crf": 23
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `video_path` | string | ✅ | Path to source video |
| `output_format` | string | ❌ | Output format: `mp4`, `webm`, `mov` (default: `mp4`) |
| `width` | integer | ❌ | Target width (e.g. 1080 for vertical) |
| `height` | integer | ❌ | Target height (e.g. 1920 for vertical) |
| `crf` | integer | ❌ | Quality: 18 (high) to 28 (low). Default: 23 |
| `output_filename` | string | ❌ | Output filename |

#### `create_vertical_clip`
Create a 9:16 vertical clip from landscape video with blur background and center crop. Perfect for TikTok/Reels/Shorts.

```bash
mcp-cli call heygen-content-mcp create_vertical_clip '{
  "video_path": "/path/to/podcast.mp4",
  "start": 120,
  "end": 180,
  "output_filename": "vertical_clip.mp4",
  "add_captions": true,
  "transcript_path": "/path/to/transcript.json"
}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `video_path` | string | ✅ | Path to source video |
| `start` | number | ✅ | Start time in seconds |
| `end` | number | ✅ | End time in seconds |
| `output_filename` | string | ❌ | Output filename |
| `add_captions` | boolean | ❌ | Burn captions from transcript (default: `false`) |
| `transcript_path` | string | ❌ | Path to transcript.json (if `add_captions=true`) |

---

### 📁 File Management

#### `list_videos`
List videos in the output directory.

```bash
mcp-cli call heygen-content-mcp list_videos '{"limit": 20}'
```

#### `get_video_info`
Get metadata about a video file (duration, resolution, codec, etc.).

```bash
mcp-cli call heygen-content-mcp get_video_info '{"video_path": "/path/to/video.mp4"}'
```

#### `delete_video`
Delete a video file from the output directory.

```bash
mcp-cli call heygen-content-mcp delete_video '{"filename": "video.mp4"}'
```

#### `get_status`
Get MCP server status: available features, tool count, server health.

```bash
mcp-cli call heygen-content-mcp get_status '{}'
```

---

## Workflow: Edit a Video from Podcast/Competitor Content <a name="workflow"></a>

### Step 1: Get the Source Video

Download the podcast or competitor video you want to repurpose.

```bash
# Example: download a YouTube video
yt-dlp -o "/path/to/source.mp4" "https://youtube.com/watch?v=XXXXX"
```

### Step 2: Transcribe the Video

Run Whisper transcription to get word-level timestamps.

```bash
mcp-cli call heygen-content-mcp transcribe '{
  "file_path": "/path/to/source.mp4",
  "model": "medium.en",
  "language": "en"
}'
```

**Output:** `transcript.json` with word-level timestamps

### Step 3: AI Finds Best Moments

Use OpenRouter AI to analyze the transcript and find the most engaging moments.

```bash
mcp-cli call heygen-content-mcp analyze_transcript '{
  "transcript": "/path/to/transcript.json",
  "max_clips": 10,
  "instruction": "Extract the 10 most engaging moments for social media. Focus on funny, insightful, or emotional moments.",
  "min_duration": 15,
  "max_duration": 120
}'
```

**Output:** JSON with `clips` array containing timestamps and reasons

### Step 4: Cut the Clips

Use the timestamps from Step 3 to cut clips from the original video.

```bash
# Cut multiple clips at once
mcp-cli call heygen-content-mcp cut_multiple_clips '{
  "video_path": "/path/to/source.mp4",
  "windows": [
    {"start": 120.5, "end": 180.2, "label": "funny_moment"},
    {"start": 300.0, "end": 360.0, "label": "insight"},
    {"start": 450.0, "end": 510.0, "label": "call_to_action"}
  ]
}'
```

### Step 5: Create Vertical Clips (for TikTok/Reels/Shorts)

Convert landscape clips to 9:16 vertical format with blur background.

```bash
mcp-cli call heygen-content-mcp create_vertical_clip '{
  "video_path": "/path/to/source.mp4",
  "start": 120.5,
  "end": 180.2,
  "output_filename": "vertical_funny_moment.mp4",
  "add_captions": true,
  "transcript_path": "/path/to/transcript.json"
}'
```

### Step 6: Add Branding

Add your logo, text overlay, or watermark with timestamp control.

```bash
# Add brand name at bottom
mcp-cli call heygen-content-mcp add_text_overlay '{
  "video_path": "/path/to/vertical_funny_moment.mp4",
  "text": "@yourbrand",
  "position": "bottom",
  "font_size": 28,
  "font_color": "white",
  "bg_color": "#000000CC",
  "output_filename": "branded_funny_moment.mp4"
}'

# Add caption at top
mcp-cli call heygen-content-mcp add_text_overlay '{
  "video_path": "/path/to/branded_funny_moment.mp4",
  "text": "This is gold",
  "position": "top",
  "font_size": 36,
  "font_color": "yellow",
  "output_filename": "final_funny_moment.mp4"
}'

# Add semi-transparent watermark throughout
mcp-cli call heygen-content-mcp add_video_watermark '{
  "video_path": "/path/to/final_funny_moment.mp4",
  "text": "@yourbrand",
  "position": "bottom-right",
  "font_size": 20,
  "opacity": 0.3
}'

# Overlay a logo corner bug
mcp-cli call heygen-content-mcp add_logo_to_video '{
  "video_path": "/path/to/final_funny_moment.mp4",
  "logo_path": "/path/to/logo.png",
  "position": "top-right",
  "scale": 0.08,
  "opacity": 0.85
}'
```

### Step 6b: Competitor Comparison (Split Screen)

Use `split_screen_video` to create comparison videos — competitor content above, your product/brand video below.

```bash
# Vertical split: competitor on TOP, your demo on BOTTOM
mcp-cli call heygen-content-mcp split_screen_video '{
  "video_a": "/path/to/competitor_clip.mp4",
  "video_b": "/path/to/mirorpay_demo.mp4",
  "layout": "vertical",
  "start_a": 0,
  "start_b": 0,
  "duration_a": 30,
  "duration_b": 30,
  "audio_source": "a"
}'

# Then add branding/text overlay on top of the split
mcp-cli call heygen-content-mcp add_text_overlay '{
  "video_path": "/path/to/split_video_xxx.mp4",
  "text": "Competitor vs MirorPay",
  "position": "top",
  "font_size": 32,
  "font_color": "white",
  "bg_color": "#000000CC",
  "start_time": 0,
  "end_time": 5
}'
```

### Step 7: Generate TTS Voiceover (Optional)

Add a custom voiceover intro or outro.

```bash
# Generate intro voiceover
mcp-cli call heygen-content-mcp tts '{
  "text": "Welcome to our channel! Here are the best moments from this week.",
  "voice": "af_heart",
  "output_filename": "intro.wav"
}'

# Extract original audio, replace with new audio
mcp-cli call heygen-content-mcp extract_audio '{
  "video_path": "/path/to/clip.mp4",
  "output_filename": "original_audio.wav"
}'
```

### Step 8: List & Verify Output

Check your generated clips.

```bash
mcp-cli call heygen-content-mcp list_videos '{"limit": 10}'
```

---

## Complete Example: Repurpose a Podcast Episode

```bash
# 1. Download podcast
yt-dlp -o "/tmp/podcast.mp4" "https://youtube.com/watch?v=XXXXX"

# 2. Transcribe
mcp-cli call heygen-content-mcp transcribe '{
  "file_path": "/tmp/podcast.mp4",
  "model": "medium.en"
}'

# 3. AI finds best moments
mcp-cli call heygen-content-mcp analyze_transcript '{
  "transcript": "/tmp/transcript.json",
  "max_clips": 5,
  "instruction": "Extract 5 most engaging moments for TikTok. Focus on funny or controversial takes."
}'

# 4. Cut clips (using timestamps from step 3)
mcp-cli call heygen-content-mcp cut_multiple_clips '{
  "video_path": "/tmp/podcast.mp4",
  "windows": [
    {"start": 45.2, "end": 78.5, "label": "hot_take_1"},
    {"start": 156.0, "end": 198.0, "label": "funny_moment"},
    {"start": 312.5, "end": 345.0, "label": "insight"},
    {"start": 480.0, "end": 520.0, "label": "controversial"},
    {"start": 600.0, "end": 640.0, "label": "call_to_action"}
  ]
}'

# 5. Create vertical clips for TikTok
for clip in /tmp/output/hot_take_1.mp4 /tmp/output/funny_moment.mp4 /tmp/output/insight.mp4; do
  mcp-cli call heygen-content-mcp create_vertical_clip "{
    \"video_path\": \"$clip\",
    \"start\": 0,
    \"end\": 999,
    \"add_captions\": true,
    \"transcript_path\": \"/tmp/transcript.json\"
  }"
done

# 6. Add branding to each
for clip in /tmp/output/vertical_*.mp4; do
  mcp-cli call heygen-content-mcp add_text_overlay "{
    \"video_path\": \"$clip\",
    \"text\": \"@yourbrand\",
    \"position\": \"bottom\",
    \"font_size\": 28,
    \"font_color\": \"white\",
    \"bg_color\": \"0x00000080\"
  }"
done

# 7. List final output
mcp-cli call heygen-content-mcp list_videos '{"limit": 20}'
```

---

## Tips & Best Practices <a name="tips"></a>

### Transcription
- Use `medium.en` for English content (good balance of speed/accuracy)
- Use `large-v3` for non-English or when accuracy is critical
- Use `tiny.en` for quick drafts

### Clip Cutting
- Set `reencode: true` for frame-accurate cuts (important for captions)
- Set `reencode: false` for faster cuts (uses stream copy)

### Vertical Clips
- Always use `create_vertical_clip` for TikTok/Reels/Shorts
- Enable `add_captions: true` for better engagement
- The blur background effect makes landscape content work vertically

### Text Overlays
- Use `bg_color: "0x00000080"` for semi-transparent background
- Keep text short (under 30 characters for mobile readability)
- Use `position: "bottom"` for brand names, `position: "top"` for captions

### AI Analysis
- Provide specific `instruction` for better clip selection
- Adjust `min_duration` and `max_duration` based on platform:
  - TikTok: 15-60 seconds
  - Instagram Reels: 15-30 seconds
  - YouTube Shorts: 30-60 seconds
  - LinkedIn: 30-120 seconds

### Output Directory
- All generated files go to: `/Users/vakandi/Documents/mcps_server/heygen-content-mcp/HeyGen/output/`
- Use `list_videos` to see what's been generated
- Use `get_video_info` to check metadata before posting
