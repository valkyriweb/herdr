#!/usr/bin/env python3
"""Resume pi, pii, Claude Code and Codex sessions in herdr-core (hc) terminals after a restart.

hc keeps terminals across a core or host restart, but recovery starts a shell and
forgets the original argv. Run "save" while the agents are live, and "restore"
after the restart: each recovered shell gets a command that resumes the same
session in the same folder.

  hc-resume save [--file F]
  hc-resume show [--file F]
  hc-resume restore [--file F] [--dry-run] [--only t1,t2] [--gap S]

The snapshot defaults to ~/.local/state/hc-resume/<HC_SESSION or default>.json.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import re
import shlex
import subprocess
import sys
import time
from pathlib import Path

HOME = Path.home()
SHELLS = {"sh", "bash", "zsh", "fish", "nu", "-sh", "-bash", "-zsh", "login"}
UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")

# Flags carried into the resume command: flag -> takes a value.
KEEP = {
    "pi": {"--model": True, "--provider": True, "--thinking": True},
    "claude": {
        "--model": True,
        "--effort": True,
        "--permission-mode": True,
        "--add-dir": True,
        "--dangerously-skip-permissions": False,
    },
    "codex": {
        "-m": True,
        "--model": True,
        "-c": True,
        "--config": True,
        "-s": True,
        "--sandbox": True,
        "--dangerously-bypass-approvals-and-sandbox": False,
        "--no-daemon": False,
    },
}


def run(argv: list[str]) -> str:
    return subprocess.run(argv, check=True, capture_output=True, text=True).stdout


def hc_json(*args: str):
    return json.loads(run(["hc", *args, "--json"]))


def tokens(argv: list[str]) -> list[str]:
    """argv, with sh -c script strings split so wrapped launches are visible too."""
    out: list[str] = []
    for arg in argv:
        if " " in arg or "\n" in arg:
            try:
                out.extend(shlex.split(arg, comments=True))
            except ValueError:
                out.append(arg)
        else:
            out.append(arg)
    return out


def harness_of(agent: str | None, toks: list[str]) -> str | None:
    names = [os.path.basename(t) for t in toks]
    if "pii" in names:
        return "pii"
    if agent in ("pi", "claude", "codex"):
        return agent
    for name in names:
        if name in ("pi", "claude", "codex"):
            return name
    return None


def flag_value(toks: list[str], *flags: str) -> str | None:
    for i, t in enumerate(toks):
        for f in flags:
            if t == f and i + 1 < len(toks):
                return toks[i + 1]
            if t.startswith(f + "="):
                return t.split("=", 1)[1]
    return None


def kept_flags(kind: str, toks: list[str]) -> list[str]:
    """Known option flags from the original launch; prompts and session flags are dropped."""
    keep = KEEP["pi" if kind == "pii" else kind]
    out: list[str] = []
    i = 0
    while i < len(toks):
        t = toks[i]
        name = t.split("=", 1)[0]
        if name in keep:
            if keep[name] and "=" not in t and i + 1 < len(toks):
                out += [t, toks[i + 1]]
                i += 2
                continue
            out.append(t)
        i += 1
    return out


def pi_slug(cwd: str) -> str:
    return "--" + cwd.strip("/").replace("/", "-") + "--"


def agent_dir(pid: int | None) -> str | None:
    """PI_CODING_AGENT_DIR from a pi process's launch environment; only that one value is read."""
    if not pid:
        return None
    try:
        out = run(["ps", "eww", "-o", "command=", "-p", str(pid)])
    except (OSError, subprocess.CalledProcessError):
        return None
    m = re.search(r"(?:^|\s)PI_CODING_AGENT_DIR=(\S+)", out)
    return m.group(1) if m else None


def pi_newest(kind: str, cwd: str, adir: str | None) -> str | None:
    # pii keeps its own profile (the pii launcher's default); pi uses ~/.pi/agent.
    root = Path(adir or (HOME / ".pi/pii" if kind == "pii" else HOME / ".pi/agent")) / "sessions"
    for c in dict.fromkeys([cwd, os.path.realpath(cwd)]):
        files = sorted((root / pi_slug(c)).glob("*.jsonl"), key=lambda p: p.stat().st_mtime)
        if files:
            return str(files[-1])
    return None


def claude_session(pid: int | None) -> str | None:
    if not pid:
        return None
    try:
        return json.loads((HOME / ".claude/sessions" / f"{pid}.json").read_text()).get("sessionId")
    except (OSError, ValueError):
        return None


def codex_session(pid: int | None) -> str | None:
    if not pid:
        return None
    try:
        out = run(["lsof", "-Fn", "-p", str(pid)])
    except (OSError, subprocess.CalledProcessError):
        return None
    for line in out.splitlines():
        if line.startswith("n") and "/rollout-" in line and line.endswith(".jsonl"):
            ids = UUID.findall(os.path.basename(line))
            if ids:
                return ids[-1]
    return None


def last_model(path: str | None, key) -> str | None:
    if not path:
        return None
    model = None
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                if '"model"' not in line:
                    continue
                try:
                    model = key(json.loads(line)) or model
                except (ValueError, AttributeError, TypeError):
                    pass
    except OSError:
        return None
    return model


def claude_model(sid: str) -> str | None:
    files = glob.glob(str(HOME / ".claude/projects" / "*" / f"{sid}.jsonl"))
    return last_model(files[0] if files else None, lambda r: (r.get("message") or {}).get("model"))


def codex_meta(sid: str) -> tuple[str | None, str | None]:
    """(provider, model) recorded in the session's rollout: resuming must name both or codex refuses."""
    files = glob.glob(str(HOME / ".codex/sessions/*/*/*" / f"rollout-*{sid}.jsonl"))
    if not files:
        return None, None
    provider = None
    try:
        with open(files[0], encoding="utf-8", errors="replace") as f:
            first = json.loads(f.readline() or "{}")
            provider = (first.get("payload") or {}).get("model_provider")
    except (OSError, ValueError):
        pass
    model = last_model(files[0], lambda r: r.get("payload", {}).get("model") if r.get("type") == "turn_context" else None)
    return provider, model


def plan(term: dict, proc_args: list[str]) -> dict | None:
    """Decide one terminal's resume command from what hc and the OS report. Pure apart from session lookups."""
    fg = term.get("foreground") or {}
    hc_toks, ps_toks = tokens(term.get("argv") or []), tokens(proc_args)
    kind = harness_of((term.get("status") or {}).get("app") or term.get("agent"), hc_toks + ps_toks)
    if not kind:
        return None
    # The running process's own argv is the best record, unless it hid it (pi retitles itself).
    toks = ps_toks if harness_of(None, ps_toks) and any(ps_toks[1:]) else hc_toks
    cwd = fg.get("cwd") or term.get("cwd")
    pid = fg.get("pid")
    labels = term.get("labels") or {}
    sid, source = None, None
    if kind in ("pi", "pii"):
        adir = agent_dir(pid)
        if adir and os.path.basename(adir.rstrip("/")) == "pii":
            kind = "pii"
        sid = flag_value(toks, "--session-id", "--session")
        source = "argv"
        if not sid and labels.get("harbor.harness-session-id"):
            sid, source = labels["harbor.harness-session-id"], "label"
        if not sid:
            sid, source = pi_newest(kind, cwd, adir), "newest-in-cwd"
        argv = [kind, "--session", sid] + kept_flags(kind, toks) if sid else None
    elif kind == "claude":
        sid, source = claude_session(pid), "claude-pid-file"
        if not sid:
            sid, source = flag_value(toks, "--session-id", "--resume", "-r"), "argv"
        flags = kept_flags("claude", toks)
        if sid and "--model" not in flags and (m := claude_model(sid)):
            flags += ["--model", m]
        argv = ["claude", "--resume", sid] + flags if sid else None
    else:
        sid, source = codex_session(pid), "codex-open-rollout"
        if not sid and "resume" in toks:
            nxt = toks[toks.index("resume") + 1 :]
            sid, source = (nxt[0] if nxt and UUID.fullmatch(nxt[0]) else None), "argv"
        flags = kept_flags("codex", toks)
        if "--no-daemon" not in flags:
            flags.append("--no-daemon")
        provider, model = codex_meta(sid) if sid else (None, None)
        if provider and not any("model_provider" in f for f in flags):
            flags += ["-c", f"model_provider={json.dumps(provider)}"]
        if model and not ({"-m", "--model"} & set(flags)):
            flags += ["-m", model]
        argv = ["codex", "resume", sid] + flags if sid else None
    return {
        "address": term["address"],
        "name": term.get("name"),
        "harness": kind,
        "session": sid,
        "source": source,
        "cwd": cwd,
        "argv": argv,
    }


def default_file() -> Path:
    base = Path(os.environ.get("XDG_STATE_HOME") or HOME / ".local/state")
    return base / "hc-resume" / f"{os.environ.get('HC_SESSION') or 'default'}.json"


def proc_args(pid: int | None) -> list[str]:
    if not pid:
        return []
    try:
        return shlex.split(run(["ps", "-o", "command=", "-p", str(pid)]).strip())
    except (subprocess.CalledProcessError, ValueError):
        return []


def previous(path: Path) -> dict:
    try:
        return {r["address"]: r for r in json.loads(path.read_text())["terminals"] if r.get("argv")}
    except (OSError, ValueError, KeyError):
        return {}


def save(path: Path) -> int:
    records, skipped = [], []
    before = previous(path)
    for t in hc_json("ls")["terminals"]:
        if t.get("state") != "live" or "/" in t["address"]:
            continue
        term = hc_json("get", t["address"])
        if is_shell(term) and t["address"] in before:
            # Recovered but not resumed yet: a re-save must not forget its agent.
            records.append(before[t["address"]])
            continue
        rec = plan(term, proc_args((term.get("foreground") or {}).get("pid")))
        if rec and rec["argv"]:
            records.append(rec)
        elif rec:
            skipped.append(rec)
    # Two agents of one harness in one folder must not both guess the same newest pi file.
    guesses = [r for r in records if r["source"] == "newest-in-cwd"]
    for r in guesses:
        if sum(1 for g in guesses if g["cwd"] == r["cwd"] and g["harness"] == r["harness"]) > 1:
            r["argv"], r["skip"] = None, "ambiguous: several pi terminals share this folder without a session id"
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps({"saved_ms": int(time.time() * 1000), "terminals": records + skipped}, indent=1))
    os.chmod(tmp, 0o600)
    tmp.replace(path)
    ok = sum(1 for r in records if r["argv"])
    print(f"saved {ok} resumable agent terminal(s) to {path}")
    for r in records + skipped:
        if not r["argv"]:
            print(f"  skip {r['address']} {r['harness']}: {r.get('skip', 'no session id found')}", file=sys.stderr)
    return 0


def is_shell(term: dict) -> bool:
    fg = term.get("foreground") or {}
    name = os.path.basename((fg.get("argv") or [fg.get("name") or ""])[0] or fg.get("name") or "")
    return name in SHELLS or (fg.get("name") or "") in SHELLS


def restore(path: Path, dry: bool, only: set[str], gap: float) -> int:
    data = json.loads(path.read_text())
    live = {t["address"]: t for t in hc_json("ls")["terminals"] if t.get("state") == "live"}
    sent = 0
    for r in data["terminals"]:
        if not r.get("argv") or (only and r["address"] not in only and r.get("name") not in only):
            continue
        if r["address"] not in live:
            print(f"skip {r['address']}: not a live terminal")
            continue
        term = hc_json("get", r["address"])
        if not is_shell(term):
            fg = (term.get("foreground") or {}).get("name")
            print(f"skip {r['address']}: running {fg!r}, not a recovery shell")
            continue
        cmd = f"cd {shlex.quote(r['cwd'])} && {shlex.join(r['argv'])}"
        print(f"{'would send' if dry else 'resume'} {r['address']} ({r['harness']} {r['session']}): {cmd}")
        if not dry:
            subprocess.run(["hc", "send", r["address"], cmd], check=True, capture_output=True)
            sent += 1
            time.sleep(gap)
    print(f"resumed {sent} terminal(s)")
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="hc-resume", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("command", choices=["save", "show", "restore"])
    p.add_argument("--file", type=Path, default=None)
    p.add_argument("--dry-run", action="store_true")
    p.add_argument("--only", default="", help="comma-separated addresses or names")
    p.add_argument("--gap", type=float, default=2.0, help="seconds between resumes")
    a = p.parse_args(argv)
    path = a.file or default_file()
    if a.command == "save":
        return save(path)
    if a.command == "show":
        print(path.read_text())
        return 0
    return restore(path, a.dry_run, {s for s in a.only.split(",") if s}, a.gap)


if __name__ == "__main__":
    sys.exit(main())
