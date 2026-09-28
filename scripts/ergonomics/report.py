#!/usr/bin/env python3
"""Call-quality metrics from the runs of one or more phases.

    python3 scripts/ergonomics/report.py base thin head          # markdown tables
    python3 scripts/ergonomics/report.py base --json             # one record per run

A condition is a phase and a model. A call is one invocation of the tool through
the shim. Help calls (--help, -h, help, capabilities, robot-docs, init, health,
--version, and a bare `jev`) are counted apart: they are reading, not driving.
A work call is valid when its exit is not 2 (usage), not 6 (input) and below 128
(killed, interrupted, or 130: declined at a confirmation no one can answer).
"""

import argparse
import collections
import json
import math
import os
import pathlib
import re
import statistics
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from ergo import ROOT, TASK, parse_transcript, read_calls  # noqa: E402

HELP_VERBS = {"help", "capabilities", "robot-docs", "init", "health"}
INVALID = {2, 6}
JEVIFY_VERBS = {"fill", "pick", "why", "route", "filter", "label", "is", "add", "sort"} | HELP_VERBS
# the jevify form each task is written for; `fill` tasks also count a pick --from
INTENDED = {"pick-from": ("pick", "--from"), "pick-files": ("pick", "--files"), "pick": ("pick", None),
            "fill": ("fill", None), "filter": ("filter", None), "filter-files": ("filter", "--files"),
            "label": ("label", None), "label-files": ("label", "--files"), "is": ("is", None),
            "why": ("why", None), "route": ("route", None), "sort": ("sort", None), "add": ("add", None)}


def verb_of(argv):
    pre = argv[: argv.index("--")] if "--" in argv else argv
    skip = False
    for a in pre:
        if skip:
            skip = False
            continue
        if a in ("--format", "-t", "--threshold", "--model"):
            skip = True
            continue
        if not a.startswith("-"):
            return a
    return None


def is_help(kind, argv):
    if kind == "thin":
        return not argv or argv[0] in ("-h", "--help")
    v = verb_of(argv)
    return (any(a in ("-h", "--help", "-V", "--version") for a in argv) or v in HELP_VERBS
            or not argv)


def valid(c):
    e = c.get("exit")
    return e is not None and e not in INVALID and e < 128


def answered(c, kind):
    """The call printed an answer: exit 0, or exit 1 where 1 is an answer (is: no;
    filter: kept none). An abstention (3) is valid but not an answer."""
    e = c.get("exit")
    return e == 0 or (e == 1 and kind == "jevify" and verb_of(c.get("argv") or []) in ("is", "filter"))


def error_text(c):
    """The error the call printed: stderr, or the JSON envelope's error on stdout."""
    msg = (c.get("stderr") or "").strip()
    out = (c.get("stdout") or "").strip()
    if out.startswith("{"):
        try:
            err = json.loads(out).get("error") or {}
            parts = [err.get("kind"), err.get("message"), err.get("hint"), err.get("example")]
            msg = (msg + "\n" + " | ".join(str(p) for p in parts if p)).strip()
        except ValueError:
            pass
    return msg


def hint_tokens(text):
    """Flags and verbs the error's hint or example names."""
    lines = [l for l in text.splitlines() if re.search(r"hint|try|example|\|", l, re.I)] or [text]
    t = " ".join(lines)
    return set(re.findall(r"--?[a-zA-Z][\w-]*", t)) | {w for w in re.findall(r"\b[a-z-]+\b", t)
                                                     if w in JEVIFY_VERBS}


def friction(c, kind):
    """A short name for why a work call was invalid."""
    e, argv, err = c.get("exit"), c.get("argv") or [], error_text(c)
    v = verb_of(argv) if kind == "jevify" else "jev"
    if c.get("killed") or e is None:
        return f"{v}: killed or never returned (stdin={c.get('stdin')})"
    if e == 130:
        return f"{v}: declined, no one to confirm (exit 130)"
    m = re.search(r"unexpected argument '([^']*)'", err)
    if m:
        x = m.group(1)
        pos = [a for a in argv[1:] if not a.startswith("-")]
        if " " not in x and not x.startswith("/") and len(pos) > 2:
            return f"{v}: unquoted multi-word description (unexpected argument)"
        what = "a flag it does not have" if x.startswith("-") else (
            "a second path" if "/" in x else ("`-` as an argument" if x == "-" else "an extra argument"))
        return f"{v}: {what} (unexpected argument)"
    if "required arguments were not provided" in err:
        return f"{v}: required argument missing"
    m = re.search(r"error: ([^\n]{0,70})", err) or re.search(r"jev: ([^\n]{0,70})", err)
    detail = m.group(1) if m else (err.splitlines()[0][:70] if err else "(nothing printed on stderr)")
    detail = re.sub(r"/\S+", "<path>", detail)
    detail = re.sub(r"\d+", "N", detail)
    return f"{v}: exit {e}, {detail}"


def run_record(run):
    meta = json.loads((run / "meta.json").read_text())
    calls = read_calls(run)
    kind = meta["kind"]
    work = [c for c in calls if not is_help(kind, c.get("argv") or [])]
    first_ok = next((i + 1 for i, c in enumerate(work) if answered(c, kind)), None)
    hints = []
    for i, c in enumerate(calls):
        if is_help(kind, c.get("argv") or []) or valid(c):
            continue
        err = error_text(c)
        nxt = calls[i + 1] if i + 1 < len(calls) else None
        toks = hint_tokens(err) - set(c.get("argv") or [])
        followed = None
        if nxt is not None:
            nargv = nxt.get("argv") or []
            # the generic hint names only --help and capabilities --json: following it
            # means reading, and its --json is not a flag the failed call was missing
            generic = "see `jevify --help`" in err
            followed = is_help(kind, nargv) if generic else bool(toks & set(nargv))
        hints.append({"has_hint": bool(re.search(r"hint|try:|example", err, re.I)), "next": nxt is not None,
                      "followed": followed, "fixed": bool(nxt is not None and valid(nxt))})
    want_verb, want_flag = INTENDED[meta["verb"]]
    used = any(verb_of(c.get("argv") or []) == want_verb and (want_flag is None or want_flag in (c.get("argv") or []))
               for c in work) or (meta["verb"] == "fill" and any(
                   verb_of(c.get("argv") or []) == "pick" and "--from" in (c.get("argv") or []) for c in work))
    transcript = (run / "transcript.jsonl").read_text()
    _, bash, _ = parse_transcript(transcript)
    # runs the environment spoiled, set apart rather than scored: a backend that
    # refused (exit 4: quota, concurrency) or a shell that could not write a here-document
    spoiled = [why for why, hit in (
        ("backend", any(c.get("exit") == 4 for c in calls)),
        ("heredoc", "can't create temp file for here document" in transcript)) if hit]
    return {**meta, "help_calls": len(calls) - len(work), "work_calls": len(work),
            "spoiled": spoiled,
            "bypass": sum(1 for b in bash if str(ROOT / "bin") in b),
            "first_valid": valid(work[0]) if work else None,
            "first_exit": work[0].get("exit") if work else None,
            "calls_to_first_ok": first_ok, "exits": [c.get("exit") for c in work],
            "invalid": sum(1 for c in work if not valid(c)), "hints": hints,
            "used_intended": used if kind == "jevify" else None,
            "frictions": [(friction(c, kind), c.get("argv"), c.get("stdin")) for c in work if not valid(c)]}


def wilson(k, n, z=1.96):
    if n == 0:
        return (float("nan"), float("nan"))
    p = k / n
    d = 1 + z * z / n
    c = p + z * z / (2 * n)
    r = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n))
    return ((c - r) / d, (c + r) / d)


def pct(k, n):
    return "–" if n == 0 else f"{100 * k / n:.0f}%"


def med(xs):
    xs = [x for x in xs if x is not None]
    return "–" if not xs else (f"{statistics.median(xs):.1f}" if isinstance(xs[0], float) else
                               f"{statistics.median(xs):g}")


def summary(rs):
    called = [r for r in rs if r["work_calls"]]
    fv = sum(1 for r in called if r["first_valid"])
    lo, hi = wilson(fv, len(called))
    hints = [h for r in rs for h in r["hints"] if h["next"]]
    return {
        "runs": len(rs), "called": len(called),
        "first_valid": f"{pct(fv, len(called))} [{100 * lo:.0f}–{100 * hi:.0f}]" if called else "–",
        "first_valid_k": fv,
        "to_ok": med([r["calls_to_first_ok"] for r in called]),
        "never_ok": pct(sum(1 for r in called if r["calls_to_first_ok"] is None), len(called)),
        "calls": f"{sum(r['work_calls'] for r in rs) / max(len(rs), 1):.2f}",
        "invalid": pct(sum(r["invalid"] for r in rs), sum(r["work_calls"] for r in rs)),
        "hint_follow": f"{sum(1 for h in hints if h['followed'])}/{len(hints)}",
        "hint_fixed": f"{sum(1 for h in hints if h['fixed'])}/{len(hints)}",
        "intended": pct(sum(1 for r in rs if r["used_intended"]), len(rs)) if rs[0]["kind"] == "jevify" else "–",
        "correct": pct(sum(1 for r in rs if r["correct"]), len(rs)),
        "turns": med([r["turns"] for r in rs]),
        "in_tok": med([r["input_tokens"] for r in rs]), "out_tok": med([r["output_tokens"] for r in rs]),
        "cost": f"{sum(r['cost_usd'] for r in rs):.2f}",
        "short": f"{sum(1 for r in rs if r['calls_logged'] < r['calls_typed'])}/{len(rs)}",
        "bypass": sum(r["bypass"] for r in rs),
    }


COLS = [("runs", "runs"), ("called", "called"), ("first_valid", "first call valid [95% CI]"),
        ("to_ok", "calls to 1st answer (median)"), ("never_ok", "never answered"),
        ("calls", "work calls/run"), ("invalid", "invalid calls"), ("hint_follow", "hint followed"),
        ("hint_fixed", "next call valid"), ("intended", "intended verb"), ("correct", "correct"),
        ("turns", "turns"), ("in_tok", "input tok"), ("out_tok", "output tok"), ("cost", "USD"),
        ("short", "runs log < typed"), ("bypass", "bypass cmds")]


def table(rows, keys):
    head = "| " + " | ".join(k for k in keys) + " | " + " | ".join(c[1] for c in COLS) + " |"
    sep = "|" + "---|" * (len(keys) + len(COLS))
    out = [head, sep]
    for k, s in rows:
        out.append("| " + " | ".join(k) + " | " + " | ".join(str(s[c[0]]) for c in COLS) + " |")
    return "\n".join(out)


def shown(argv):
    """argv as a shell line, quoted where the agent's shell kept a word together,
    with the run's own directory written $RUN so no local path is printed."""
    s = " ".join(x if x and not re.search(r"[\s'\"|;&]", x) else repr(x) for x in argv)
    s = s.replace(str(ROOT / "runs"), "$RUNS")
    s = re.sub(r"\$RUNS/[^/\s'\"]+/[^/\s'\"]+", "$RUN", s)
    s = s.replace(os.path.expanduser("~"), "~")
    return s[:160].replace("|", "\\|")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("phases", nargs="+")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--examples", type=int, default=2)
    ap.add_argument("--all", action="store_true", help="keep the runs the environment spoiled")
    a = ap.parse_args()
    recs = []
    for ph in a.phases:
        for run in sorted((ROOT / "runs" / ph).iterdir()):
            if (run / "meta.json").exists():
                recs.append(run_record(run))
    if a.json:
        for r in recs:
            print(json.dumps(r))
        return
    if not a.all:
        print("## Runs set apart\n")
        for ph in a.phases:
            sp = collections.Counter(w for r in recs if r["phase"] == ph for w in r["spoiled"])
            n = sum(1 for r in recs if r["phase"] == ph)
            k = sum(1 for r in recs if r["phase"] == ph and r["spoiled"])
            print(f"- {ph}: {k} of {n} runs (backend refused: {sp['backend']}, here-document: {sp['heredoc']})")
        print()
        recs = [r for r in recs if not r["spoiled"]]
    conds = collections.OrderedDict()
    for r in recs:
        conds.setdefault((r["phase"], r["model"]), []).append(r)

    print("## By condition\n")
    print(table([((p, m), summary(rs)) for (p, m), rs in conds.items()], ["phase", "model"]))

    print("\n## By verb\n")
    rows = []
    for (p, m), rs in conds.items():
        by = collections.OrderedDict()
        for r in sorted(rs, key=lambda r: list(TASK).index(r["task"])):
            by.setdefault(r["verb"], []).append(r)
        rows += [((p, m, v), summary(x)) for v, x in by.items()]
    print(table(rows, ["phase", "model", "verb"]))

    print("\n## Exit codes of work calls\n")
    for (p, m), rs in conds.items():
        c = collections.Counter(str(e) for r in rs for e in r["exits"])
        print(f"- {p} / {m}: " + ", ".join(f"{k} x{v}" for k, v in sorted(c.items(), key=lambda kv: -kv[1])))

    print("\n## Friction: why work calls were invalid\n")
    for (p, m), rs in conds.items():
        f = collections.defaultdict(list)
        for r in rs:
            for name, argv, stdin in r["frictions"]:
                f[name].append((r["rid"], argv))
        if not f:
            continue
        print(f"### {p} / {m}\n")
        print("| n | pattern | example argv |\n|---:|---|---|")
        for name, ex in sorted(f.items(), key=lambda kv: -len(kv[1])):
            eg = "<br>".join("`" + shown(argv) + "`" for _, argv in ex[: a.examples])
            print(f"| {len(ex)} | {name} | {eg} |")
        print()


if __name__ == "__main__":
    main()
