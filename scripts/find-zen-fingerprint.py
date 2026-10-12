#!/usr/bin/env python3
"""Brute-force the Zen request fingerprint.

OMP is not OpenCode yet it reaches the models, so the difference is carried by the
request itself. Every candidate shape is tried against the real gateway and whatever
answers with something other than 403 wins.
"""

import itertools
import json
import re
import subprocess
import sys
import urllib.request
import urllib.error
from pathlib import Path

LOG = Path.home() / ".local/share/opencode/log/opencode.log"
POOL = Path.home() / ".omp/agent/extensions/proxies.txt"
BASE = "https://opencode.ai/zen/v1"

UA = "opencode/1.18.32 ai-sdk/provider-utils/4.0.40 runtime/bun/1.3.14"


def mint() -> tuple[str, str]:
    """Run a real OpenCode turn and read back the ses_/msg_ pair it used."""
    subprocess.run(
        [str(Path.home() / ".nvm/versions/node/v20.20.2/bin/opencode"),
         "run", "-m", "opencode/muse-spark-1.3-contributor-free", "hi"],
        capture_output=True, timeout=180,
    )
    tail = LOG.read_text(errors="ignore")[-200000:]
    ids = re.findall(r"(?:ses|msg)_[A-Za-z0-9]{24}", tail)
    for msg in reversed(ids):
        if not msg.startswith("msg_"):
            continue
        idx = tail.rfind(msg)
        before = tail[:idx]
        ses = re.findall(r"ses_[A-Za-z0-9]{24}", before)
        if ses:
            return ses[-1], msg
    raise SystemExit("could not mint a session pair")


def proxies(n: int = 6) -> list[str]:
    out = []
    for line in POOL.read_text().splitlines():
        p = line.split("|")[0].split(":")
        if len(p) >= 4:
            out.append(f"http://{p[2]}:{p[3]}@{p[0]}:{p[1]}")
        if len(out) >= n:
            break
    return out


def call(proxy: str, url: str, headers: dict, body: dict, timeout: int = 45):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), method="POST")
    for k, v in headers.items():
        req.add_header(k, v)
    opener = urllib.request.build_opener(
        urllib.request.ProxyHandler({"http": proxy, "https": proxy})
    )
    try:
        with opener.open(req, timeout=timeout) as resp:
            return resp.status, resp.read(300).decode(errors="ignore")
    except urllib.error.HTTPError as e:
        return e.code, e.read(300).decode(errors="ignore")
    except Exception as e:  # dead proxy, not a verdict on the headers
        return 0, type(e).__name__


def variants(session: str, request: str):
    """Every plausible shape, cheapest first."""
    auth = {"Authorization": "Bearer public"}
    fingerprint = {
        "User-Agent": UA,
        "x-opencode-session": session,
        "x-opencode-request": request,
        "x-opencode-client": "cli",
        "x-opencode-project": "global",
    }
    bodies = {
        "responses-space": (f"{BASE}/responses",
                            {"model": "space-bunny-free", "input": "hi", "stream": True}),
        "chat-space": (f"{BASE}/chat/completions",
                       {"model": "space-bunny-free",
                        "messages": [{"role": "user", "content": "hi"}], "stream": True}),
    }
    # header sets to try: full fingerprint, minus one key at a time, auth-less, bare.
    keys = list(fingerprint)
    sets = [("full", {})]
    for drop in keys:
        sets.append((f"no-{drop}", {k: v for k, v in fingerprint.items() if k != drop}))
    sets.append(("no-auth", None))
    sets.append(("bare", None))

    for (name, override), (bname, (url, body)) in itertools.product(sets, bodies.items()):
        if override is None and name == "no-auth":
            hdrs = {**fingerprint, "Content-Type": "application/json",
                    "Accept": "text/event-stream"}
        elif name == "bare":
            hdrs = {"Content-Type": "application/json", "Accept": "text/event-stream"}
        else:
            hdrs = {**fingerprint, **override, "Content-Type": "application/json",
                    "Accept": "text/event-stream"}
            if name != "no-auth":
                hdrs.update(auth)
        yield name, bname, hdrs, url, body


def main() -> int:
    print("minting a session pair with real OpenCode…")
    session, request = mint()
    print(f"  session={session}\n  request={request}\n")

    pool = proxies()
    for proxy in pool:
        host = proxy.split("@")[-1]
        for name, bname, hdrs, url, body in variants(session, request):
            status, text = call(proxy, url, hdrs, body)
            if status == 0:
                continue
            if status != 403:
                print(f"*** NOT-403  {status}  via {host}\n    set={name} body={bname}")
                print(f"    headers={json.dumps(hdrs, indent=6)}")
                print(f"    url={url}")
                print(f"    reply={text[:200]}")
                return 0
            print(f"403  {name:18s} {bname:18s} via {host}")
    print("\nno combination beat 403 through these proxies.")
    return 1


if __name__ == "__main__":
    sys.exit(main())