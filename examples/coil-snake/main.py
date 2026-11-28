#!/usr/bin/env python3
"""Launcher for the snake game (the game itself is index.html).

    python3 main.py                      open the game in your web browser
    python3 main.py --screenshot a.png   render ~120 frames in a hidden browser, save a PNG, exit
    python3 main.py --self-test r.json   play scripted input in a hidden browser, save results, exit

The hidden browser runs headless, so it never takes focus.
"""
import glob
import html
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import webbrowser
from pathlib import Path

HERE = Path(__file__).resolve().parent
PAGE = HERE / "index.html"


def find_browsers():
    """Every usable Chromium-family browser, lightest headless one first."""
    found = []
    env = os.environ.get("CHROME") or os.environ.get("BROWSER_BIN")
    if env and Path(env).exists():
        found.append(env)
    home = Path.home()
    caches = [home / "Library/Caches/ms-playwright", home / ".cache/ms-playwright",
              home / "AppData/Local/ms-playwright"]
    for cache in caches:
        hits = sorted(glob.glob(str(cache / "chromium_headless_shell-*/*/chrome-headless-shell*")), reverse=True)
        hits += sorted(glob.glob(str(cache / "chromium-*/chrome-*/Chromium.app/Contents/MacOS/Chromium")), reverse=True)
        hits += sorted(glob.glob(str(cache / "chromium-*/chrome-*/chrome")), reverse=True)
        found += [h for h in hits if os.path.isfile(h) and os.access(h, os.X_OK)]
    candidates = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ]
    for c in candidates:
        if Path(c).exists():
            found.append(c)
    for name in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser",
                 "brave-browser", "microsoft-edge", "chrome", "msedge"):
        p = shutil.which(name)
        if p:
            found.append(p)
    seen, unique = set(), []
    for f in found:
        if f not in seen:
            seen.add(f)
            unique.append(f)
    return unique


def run_headless(browser, extra_args, query, timeout=40):
    """Run one headless browser on the page; None if it stalls."""
    url = PAGE.as_uri() + "?" + query
    profile = tempfile.mkdtemp(prefix="coil-snake-")
    headless = "--headless" if "headless" in Path(browser).name else "--headless=new"
    args = [browser, headless, "--disable-gpu", "--hide-scrollbars",
            "--no-first-run", "--no-default-browser-check", "--mute-audio",
            "--user-data-dir=" + profile, "--window-size=900,800"]
    args += extra_args + [url]
    try:
        return subprocess.run(args, capture_output=True, text=True, timeout=timeout)
    except (subprocess.TimeoutExpired, OSError):
        return None
    finally:
        shutil.rmtree(profile, ignore_errors=True)


def browsers_or_exit():
    found = find_browsers()
    if not found:
        sys.exit("No Chromium-family browser found (set CHROME=/path/to/chrome).")
    return found


def screenshot(out):
    out = Path(out).resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    if out.exists():
        out.unlink()
    # ~4 s of virtual time is well over 120 frames at 60 fps.
    for browser in browsers_or_exit():
        run_headless(browser, ["--screenshot=" + str(out), "--virtual-time-budget=4000"], "screenshot")
        if out.exists() and out.stat().st_size > 0:
            print("saved", out)
            return
    sys.exit("Screenshot was not saved.")


def self_test(out):
    out = Path(out).resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    result = None
    for browser in browsers_or_exit():
        proc = run_headless(browser, ["--dump-dom", "--virtual-time-budget=15000"], "self-test")
        m = re.search(r'<pre id="self-test-result">(.*?)</pre>', (proc.stdout if proc else "") or "", re.S)
        text = html.unescape(m.group(1)).strip() if m else ""
        try:
            result = json.loads(text)
            break
        except ValueError:
            continue
    if result is None:
        result = {"version": 1, "steps": [
            {"step": "move", "passed": False, "detail": "self-test did not finish in the browser"},
            {"step": "act", "passed": False, "detail": "self-test did not finish in the browser"},
            {"step": "hud", "passed": False, "detail": "self-test did not finish in the browser"},
        ]}
    out.write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))
    sys.exit(0 if all(s.get("passed") for s in result["steps"]) else 1)


def main(argv):
    if "--screenshot" in argv:
        i = argv.index("--screenshot")
        screenshot(argv[i + 1] if i + 1 < len(argv) else "screenshot.png")
    elif "--self-test" in argv:
        i = argv.index("--self-test")
        self_test(argv[i + 1] if i + 1 < len(argv) else "self-test.json")
    else:
        webbrowser.open(PAGE.as_uri())


if __name__ == "__main__":
    main(sys.argv[1:])
