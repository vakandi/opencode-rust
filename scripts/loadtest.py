#!/usr/bin/env python3
"""Concurrency probe for the Rust OpenCode server.

Fires N sessions at the server simultaneously and samples RSS while they run, so
"how many concurrent sessions fit" is answered by measurement instead of arithmetic.

Target: 100 concurrent sessions in a 6 GB container. Reports peak RSS, the implied
per-session cost, and whether the server survived.

Usage:
  scripts/loadtest.py [--n 100] [--base http://127.0.0.1:4097] [--hold 30]
                      [--budget-mb 6144] [--pid AUTO]
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

STOP = threading.Event()
PEAK_RSS_MB = 0.0
SAMPLES: list[tuple[float, float]] = []
LOCK = threading.Lock()


def rss_mb(pid: int) -> float:
    """Resident set size in MiB, or 0.0 if the process is gone."""
    try:
        with open(f"/proc/{pid}/status") as fh:  # Linux
            for line in fh:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
    except OSError:
        pass
    try:  # macOS
        out = subprocess.run(
            ["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True, timeout=5
        )
        if out.stdout.strip():
            return int(out.stdout.strip()) / 1024
    except (OSError, ValueError, subprocess.SubprocessError):
        pass
    return 0.0


def sampler(pid: int) -> None:
    global PEAK_RSS_MB
    while not STOP.is_set():
        current = rss_mb(pid)
        with LOCK:
            if current > PEAK_RSS_MB:
                PEAK_RSS_MB = current
            SAMPLES.append((time.time(), current))
        time.sleep(0.25)


def post(url: str, payload: dict, timeout: float = 20.0) -> tuple[int, dict]:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read().decode(errors="replace")
            try:
                return response.status, json.loads(body) if body else {}
            except json.JSONDecodeError:
                return response.status, {}
    except urllib.error.HTTPError as exc:
        return exc.code, {}
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        return 0, {"transport_error": str(exc)}


def get(url: str, timeout: float = 20.0) -> tuple[int, dict]:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as response:
            body = response.read().decode(errors="replace")
            try:
                return response.status, json.loads(body) if body else {}
            except json.JSONDecodeError:
                return response.status, {}
    except urllib.error.HTTPError as exc:
        return exc.code, {}
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        return 0, {"transport_error": str(exc)}


def one_session(index: int, base: str, hold: float, results: list[dict], model: str = "") -> None:
    status, session = post(f"{base}/session", {})
    if status != 200 or not session.get("id"):
        results.append({"i": index, "stage": "create", "status": status, "body": session})
        return
    sid = session["id"]
    results.append({"i": index, "stage": "create", "status": status, "sid": sid})
    payload = {"message": f"probe {index}: reply with one word."}
    if model:
        payload["model"] = model
    prompt_status, prompt_body = post(f"{base}/session/{sid}/prompt", payload)
    results.append(
        {"i": index, "stage": "prompt", "status": prompt_status, "sid": sid, "body": prompt_body}
    )
    deadline = time.time() + hold
    while time.time() < deadline and not STOP.is_set():
        time.sleep(1.0)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--n", type=int, default=100)
    parser.add_argument("--base", default="http://127.0.0.1:4097")
    parser.add_argument("--hold", type=float, default=30.0)
    parser.add_argument("--budget-mb", type=float, default=6144.0)
    parser.add_argument("--pid", default="AUTO")
    parser.add_argument("--model", default="")
    args = parser.parse_args()

    pid = args.pid
    if pid == "AUTO":
        try:
            out = subprocess.run(
                ["lsof", "-nP", f"-iTCP:{args.base.rsplit(':', 1)[1]}", "-sTCP:LISTEN", "-t"],
                capture_output=True,
                text=True,
                timeout=10,
            )
            pid = out.stdout.strip().splitlines()[0]
        except (OSError, IndexError, subprocess.SubprocessError):
            pid = ""
    if not pid:
        print("could not find the server pid; pass --pid", file=sys.stderr)
        return 1

    baseline = rss_mb(int(pid))
    print(f"server pid {pid}  baseline RSS {baseline:.0f} MiB", flush=True)
    print(f"firing {args.n} concurrent sessions, holding {args.hold:.0f}s", flush=True)

    watch = threading.Thread(target=sampler, args=(int(pid),), daemon=True)
    watch.start()

    results: list[dict] = []
    threads = [
        threading.Thread(target=one_session, args=(i, args.base, args.hold, results, args.model))
        for i in range(args.n)
    ]
    started = time.time()
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=args.hold + 60)
    elapsed = time.time() - started

    STOP.set()
    time.sleep(0.5)
    final = rss_mb(int(pid))
    alive = final > 0

    created = sum(1 for r in results if r["stage"] == "create" and r["status"] == 200)
    prompted = sum(1 for r in results if r["stage"] == "prompt" and r["status"] == 200)
    transport = [r for r in results if r.get("body", {}).get("transport_error")]
    non_2xx = [r for r in results if r["stage"] == "prompt" and r["status"] not in (200,)]

    per_session = (PEAK_RSS_MB - baseline) / args.n if args.n else 0.0
    verdict_pct = 0.0
    if args.budget_mb:
        verdict_pct = 100.0 * (PEAK_RSS_MB / args.budget_mb)

    print()
    print("=" * 66)
    print(f"  sessions requested        {args.n}")
    print(f"  sessions created          {created}")
    print(f"  prompts accepted (200)    {prompted}")
    print(f"  transport failures        {len(transport)}")
    print(f"  non-200 prompt responses  {len(non_2xx)}")
    print(f"  elapsed                   {elapsed:.1f}s")
    print("-" * 66)
    print(f"  baseline RSS              {baseline:.0f} MiB")
    print(f"  PEAK RSS                  {PEAK_RSS_MB:.0f} MiB")
    print(f"  final RSS                 {final:.0f} MiB")
    print(f"  growth per session        {per_session:.2f} MiB")
    print(f"  budget used               {verdict_pct:.1f}% of {args.budget_mb:.0f} MiB")
    print(f"  server alive after load   {alive}")
    print("=" * 66)

    if transport:
        print("  sample transport error:", transport[0]["body"]["transport_error"][:120])
    if non_2xx:
        print("  sample non-200:", json.dumps(non_2xx[0])[:200])

    if not alive:
        print("\nVERDICT: SERVER DIED under load")
        return 2
    if PEAK_RSS_MB > args.budget_mb:
        print(f"\nVERDICT: OVER BUDGET ({PEAK_RSS_MB:.0f} > {args.budget_mb:.0f} MiB)")
        return 1
    print(f"\nVERDICT: SURVIVED, {verdict_pct:.1f}% of budget")
    return 0


if __name__ == "__main__":
    signal.signal(signal.SIGINT, lambda *_: STOP.set())
    sys.exit(main())
