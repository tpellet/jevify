# /// script
# requires-python = ">=3.11"
# ///
"""The helper `evals/scorecard/run.sh` drives: the thin arm, the budget ledger, and the report.

    scorecard.py thin SET OUT                  the thin arm over evals/SET (validation|holdout)
    scorecard.py thin-variance ARM OUT         the thin arm over the variance subset (repeat|order)
    scorecard.py thin-commits CASES REPO OUT   the thin arm over one commit-subjects case file
    scorecard.py model                         the model classifier.dev answers with, one call
    scorecard.py spent DIR CAP_KEYLESS CAP_TS  classifications spent so far; exit 1 over a cap
    scorecard.py report DIR                    SCORECARD.md on stdout

The thin arm is `scripts/thinjev/jev`: the question and the raw candidate lines, one keyless
call, the top line taken as the answer. It gets what a caller of a thin wrapper has: the
records, the candidate file, the log lines, `git ls-files`, `git log --format='%h %s'`, the
tool names. It gets no lister, no excerpt, no patch, no second round and no threshold. The
wrapper takes 2..100 distinct options of at most 200 characters, so a larger input is cut the
way a shell user cuts it, `tail -n 100` for a log and `head -n 100` for any other list, each
line clipped to 200 characters and repeated lines kept once.

`thin+threshold` is the same answers with an abstention wherever the top probability is below
the threshold jevify reports for that case, so that both arms may say "nothing fits".
"""
import json
import os
import shutil
import statistics
import subprocess
import sys
import time
import urllib.request
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
THIN = REPO / "scripts" / "thinjev" / "jev"
sys.path.insert(0, str(REPO / "scripts"))
sys.path.insert(0, str(REPO / "evals" / "variance"))
import holdout_score  # noqa: E402
import validation_gold  # noqa: E402

SETS = {"validation": (REPO / "evals" / "validation", validation_gold),
        "holdout": (REPO / "evals" / "holdout", holdout_score)}
REPOS = {"validation": REPO / "evals" / "out" / "validation" / "repos",
         "holdout": REPO / "evals" / "out" / "holdout" / "repos"}
ABSTAIN = {"none", "unsure", "?", "ambiguous"}
MAX_OPTS, MAX_CHARS = 100, 200
WHY_Q = "Which line of this log is the root cause of the failure?"


# ---------------------------------------------------------------- the thin arm

def u16(s):
    return len(s.encode("utf-16-le")) // 2


def clip(s):
    while u16(s) > MAX_CHARS:
        s = s[:-1]
    return s


def prepare(lines, tail=False):
    """Options from raw lines: non-blank, cut to 100 (tail for a log, head otherwise), clipped,
    each kept once. Returns the options, the 1-based source line of each, and the count before
    the cut."""
    numbered = [(i, ln.strip()) for i, ln in enumerate(lines, 1) if ln.strip()]
    full = len(numbered)
    numbered = numbered[-MAX_OPTS:] if tail else numbered[:MAX_OPTS]
    opts, back, seen = [], [], set()
    for i, text in numbered:
        text = clip(text)
        if text not in seen:
            seen.add(text)
            opts.append(text)
            back.append(i)
    return opts, back, full


def thin(question, opts):
    """One call of the thin wrapper: (ranked [(p, option)], elapsed ms, error)."""
    if len(opts) < 2:
        return [], 0, "fewer than 2 options"
    env = {k: v for k, v in os.environ.items() if not k.startswith("TYPESAFE_")}
    started = time.monotonic()
    proc = subprocess.run([sys.executable, str(THIN), clip_question(question)],
                          input="\n".join(opts) + "\n", capture_output=True, text=True,
                          env=env, timeout=120, check=False)
    ms = int((time.monotonic() - started) * 1000)
    if proc.returncode != 0:
        return [], ms, proc.stderr.strip()[:200]
    ranked = []
    for line in proc.stdout.splitlines():
        p, _, opt = line.partition("\t")
        ranked.append((float(p), opt))
    return ranked, ms, None


def clip_question(q):
    return q if u16(q) <= 32_000 else q[-30_000:]


def git_lines(cwd, *args):
    return subprocess.run(["git", "-C", str(cwd), *args], capture_output=True, text=True,
                          check=True, timeout=600).stdout.splitlines()


def listing(kind, cwd):
    """What a shell user lists for a kind: files, their directories, or the log."""
    if kind == "commit":
        return git_lines(cwd, "log", "--abbrev=7", "--format=%h %s")
    files = git_lines(cwd, "ls-files")
    if kind == "dir":
        dirs = set()
        for f in files:
            parts = f.split("/")[:-1]
            for k in range(1, len(parts) + 1):
                dirs.add("/".join(parts[:k]))
        return sorted(dirs)
    return files


def path_tools():
    names = set()
    for d in os.environ.get("PATH", "").split(":"):
        try:
            for entry in os.scandir(d):
                if entry.is_file() and os.access(entry.path, os.X_OK):
                    names.add(entry.name)
        except OSError:
            continue
    return sorted(names)


def marker(argv):
    for a in argv:
        if a.startswith("@{") and a.endswith("}"):
            kind, _, desc = a[2:-1].partition(":")
            return kind, desc
    return None, None


def thin_case(root, case, group, repos):
    """The thin arm on one case (or one record of a grouped run): a list of rows."""
    verb, argv = case["verb"], case["argv"]
    row = {"id": case["id"], "verb": verb, "arm": "thin", "backend": "classifier",
           "decision": "not_run", "score": None, "requests": 0, "elapsed_ms": None,
           "options": None, "options_full": None, "reason": None}
    if verb in ("filter", "label"):
        rows = []
        for member in group:
            r = dict(row, id=member["id"])
            record = member["record"]
            if verb == "filter":
                q = f"Record: {record}\nStatement: {argv[1]}\nIs the statement true of the record?"
                ranked, ms, err = thin(q, ["yes", "no"])
                pick = {"yes": "keep", "no": "drop"}
            else:
                ranked, ms, err = thin(f"Record: {record}\nWhich label fits this record?",
                                       member["labels"])
                pick = {}
            r.update(elapsed_ms=ms, requests=1, options=2 if verb == "filter" else
                     len(member["labels"]), reason=err)
            if ranked:
                r["decision"] = pick.get(ranked[0][1], ranked[0][1])
                r["score"] = ranked[0][0]
            rows.append(r)
        return rows
    field = None
    if verb == "is":
        context = (root / case["context"]).read_text()
        q = f"{context}\n\nIs this true of the document above: {argv[1]}"
        opts, back, full = ["yes", "no"], None, 2
    elif verb == "route":
        missing = [t for t in case.get("requires", []) if shutil.which(t) is None]
        if missing:
            row["reason"] = f"tools absent from PATH: {', '.join(missing)}"
            return [row]
        if "inventory" in case:
            names = [t["name"] for t in json.loads((root / case["inventory"]).read_text())]
        else:
            names = path_tools()
        q = argv[1]
        opts, back, full = prepare(names)
    elif verb == "why":
        q = WHY_Q
        opts, back, full = prepare((root / case["stdin"]).read_text().splitlines(), tail=True)
    elif verb == "pick":
        q = argv[-1]
        opts, back, full = prepare((root / case["stdin"]).read_text().splitlines())
    elif "candidates" in case:
        _, q = marker(argv)
        opts, back, full = prepare((root / case["candidates"]).read_text().splitlines())
        field = 1
    else:  # a lister: fill with an @{kind:...} marker, or pick --from KIND
        if verb == "fill":
            kind, q = marker(argv)
        else:
            kind, q = argv[2], argv[-1]
        clone = repos / case["env"]["repo"].split("/")[-1]
        if not (clone / ".git").is_dir():
            row["reason"] = f"no clone under {clone}"
            return [row]
        opts, back, full = prepare(listing(kind, clone))
        field = 1 if kind == "commit" else None
    ranked, ms, err = thin(q, opts)
    row.update(elapsed_ms=ms, requests=1 if ms else 0, options=len(opts), options_full=full,
               reason=err)
    if not ranked:
        row["decision"] = "none"
        return [row]
    top_p, top = ranked[0]
    row["score"] = top_p
    if verb in ("pick", "why"):
        row["decision"] = str(back[opts.index(top)])
        row["identity"] = top
    elif field == 1:
        row["decision"] = top.split()[0][:7]
    else:
        row["decision"] = top
    return [row]


def cmd_thin(set_name, out):
    root, _ = SETS[set_name]
    cases = [json.loads(ln) for ln in (root / "cases.jsonl").read_text().splitlines() if ln.strip()]
    groups = defaultdict(list)
    for c in cases:
        if "run" in c:
            groups[c["run"]].append(c)
    done = set()
    with open(out, "w", encoding="utf-8") as sink:
        for c in cases:
            if "run" in c:
                if c["run"] in done:
                    continue
                done.add(c["run"])
            for r in thin_case(root, c, groups.get(c.get("run")), REPOS[set_name]):
                r["set"] = set_name
                sink.write(json.dumps(r) + "\n")
                sink.flush()
                print(f"  thin {r['id']:<34} {r['decision']:<30} p={r['score']} "
                      f"{r['elapsed_ms']}ms opts={r['options']}/{r['options_full']}"
                      + (f" ({r['reason']})" if r["reason"] else ""), file=sys.stderr)


def cmd_thin_variance(arm, out):
    import variance_run as vr
    units, _ = vr.load_cases()
    subset = vr.REPEAT_SUBSET if arm == "repeat" else vr.ORDER_SUBSET
    scratch = Path(out).parent / "permuted-thin"
    scratch.mkdir(parents=True, exist_ok=True)
    n = vr.TRIALS if arm == "repeat" else vr.ORDERS
    with open(out, "w", encoding="utf-8") as sink:
        for name in subset:
            unit = units[name]
            for index in range(n):
                case = dict(unit[0])
                if arm == "order":
                    field, path = vr.permute(unit, index, scratch)
                    case[field] = str(path)  # absolute: Path join keeps it
                rows = thin_case(vr.HOLDOUT, case, [dict(m) for m in unit], REPOS["holdout"])
                for r in rows:
                    member = next(m for m in unit if m["id"] == r["id"])
                    identity = member.get("record") or r.get("identity") or member["id"]
                    answer = r["decision"]
                    if case["verb"] == "pick" and answer not in ABSTAIN | {"not_run"}:
                        answer = r["identity"]
                    if case["verb"] == "why":
                        identity = member["id"]
                    sink.write(json.dumps({
                        "arm": arm, "unit": name, "index": index, "backend": "thin",
                        "id": r["id"], "verb": case["verb"], "identity": identity,
                        "answer": answer, "score": r["score"], "elapsed_ms": r["elapsed_ms"],
                        "questions": r["requests"], "reason": r["reason"]}) + "\n")
                sink.flush()
                print(f"  thin {arm} {name} #{index}", file=sys.stderr)


def cmd_thin_commits(cases_path, repo, out):
    with open(out, "w", encoding="utf-8") as sink:
        for ln in Path(cases_path).read_text().splitlines():
            if not ln.strip():
                continue
            case = json.loads(ln)
            opts, _, full = prepare(listing("commit", repo))
            ranked, ms, err = thin(case["description"], opts)
            answer = ranked[0][1].split()[0][:7] if ranked else None
            row = {"id": case["id"], "backend": "thin", "target": case["target"],
                   "claimant": case.get("claimant"), "answer": answer,
                   "p": ranked[0][0] if ranked else None, "seconds": round(ms / 1000, 2),
                   "requests": 1, "options": len(opts), "options_full": full,
                   "correct": answer == case["target"],
                   "chose_claimant": bool(case.get("claimant")) and answer == case["claimant"],
                   "reason": err}
            sink.write(json.dumps(row) + "\n")
            print(f"  thin {case['id']} {answer} p={row['p']} "
                  f"{'CORRECT' if row['correct'] else ''}", file=sys.stderr)


def cmd_model():
    body = {"items": ["a vegetable"], "dimensions": {"answer": {
        "labels": ["apple", "carrot"], "instructions": "Choose the option that best answers."}}}
    req = urllib.request.Request("https://classifier.dev/v1/classify", json.dumps(body).encode(),
                                 method="POST", headers={"content-type": "application/json",
                                                         "user-agent": "thinjev/1"})
    with urllib.request.urlopen(req, timeout=60) as r:
        print(json.load(r).get("model", "unknown"))


# ---------------------------------------------------------------- reading runs

def read_jsonl(path):
    p = Path(path)
    if not p.is_file():
        return []
    return [json.loads(ln) for ln in p.read_text().splitlines() if ln.strip()]


def manifest(set_name):
    root, _ = SETS[set_name]
    return {c["id"]: c for c in (json.loads(ln) for ln in
                                 (root / "cases.jsonl").read_text().splitlines() if ln.strip())}


def jev_calls(rows, cases):
    """One entry per call: a grouped run's rows share one call."""
    seen, calls = set(), []
    for r in rows:
        key = cases.get(r["id"], {}).get("run") or r["id"]
        if key in seen:
            continue
        seen.add(key)
        calls.append(r)
    return calls


def spent(d):
    d = Path(d)
    total = {"classifier": 0, "typesafe": 0}
    for s in SETS:
        cases = manifest(s)
        for b in total:
            total[b] += sum(r.get("questions") or 0
                            for r in jev_calls(read_jsonl(d / f"{s}-{b}.jsonl"), cases))
        total["classifier"] += sum(r.get("requests") or 0 for r in read_jsonl(d / f"{s}-thin.jsonl"))
    for arm in ("repeat", "order"):
        for b in total:
            total[b] += sum(r.get("questions") or 0 for r in read_jsonl(d / f"{arm}-{b}.jsonl"))
        total["classifier"] += sum(r.get("questions") or 0
                                   for r in read_jsonl(d / f"{arm}-thin.jsonl"))
    for half in ("scratch", "ripgrep"):
        for b in total:
            # a commit finals request carries one question per window and one for the finals
            total[b] += 2 * sum(r.get("requests") or 0
                                for r in read_jsonl(d / f"commits-{half}-{b}.jsonl"))
        total["classifier"] += len(read_jsonl(d / f"commits-{half}-thin.jsonl"))
    return total


def cmd_spent(d, cap_keyless, cap_ts):
    t = spent(d)
    print(f"classifications spent: keyless {t['classifier']} of {cap_keyless}, "
          f"TypeSafe {t['typesafe']} of {cap_ts}")
    return 1 if t["classifier"] > int(cap_keyless) or t["typesafe"] > int(cap_ts) else 0


# ---------------------------------------------------------------- scoring

def norm(x):
    return x.rstrip("/") if isinstance(x, str) and "/" in x else x


def outcome(mod, decision, gold):
    if decision in ABSTAIN:
        decision = "none" if decision == "ambiguous" else decision
    if isinstance(gold, str):
        gold = norm(gold)
    return mod.outcome(norm(decision), gold)


def with_threshold(rows, thresholds):
    out = []
    for r in rows:
        r = dict(r, arm="thin+threshold")
        thr = thresholds.get(r["id"])
        if (r["decision"] not in ABSTAIN | {"not_run"} and r["score"] is not None
                and thr is not None and r["score"] < thr):
            r["decision"] = {"filter": "unsure", "is": "unsure", "label": "?"}.get(r["verb"],
                                                                                   "none")
        out.append(r)
    return out


def pct(k, n):
    return f"{100 * k / n:.0f}%" if n else "–"


def quant(xs, q):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    return xs[min(len(xs) - 1, int(round(q * (len(xs) - 1))))]


def tally(rows, set_of, golds, cases_of):
    """Per verb: outcome counts, per-call latency, requests and classifications per case."""
    t = defaultdict(lambda: defaultdict(int))
    lat, calls_seen = defaultdict(list), set()
    for r in rows:
        s = set_of(r)
        mod = SETS[s][1]
        g = golds[s].get(r["id"])
        if g is None:
            continue
        v = r["verb"]
        o = outcome(mod, r["decision"], g["gold"])
        t[v][o] += 1
        if o == "not_run":
            continue
        t[v]["n"] += 1
        case = cases_of[s].get(r["id"], {})
        grouped = case.get("run") and not str(r.get("arm", "")).startswith("thin")
        key = (s, r.get("arm"), case["run"] if grouped else r["id"])
        if key not in calls_seen:
            calls_seen.add(key)
            lat[v].append(r.get("elapsed_ms"))
            t[v]["requests"] += r.get("requests") or 0
            t[v]["questions"] += r.get("questions") or r.get("requests") or 0
            t[v]["calls"] += 1
    return t, lat


def line(verb, arm, model, t, lat):
    n = t["n"]
    answered = t["correct"] + t["false_action"]
    abst = t["abstained_right"] + t["abstained_missed"]
    p50, p95 = quant(lat, 0.5), quant(lat, 0.95)
    return (f"| {verb} | {arm} | {model} | {n} | {pct(t['correct'], answered)} "
            f"({t['correct']}/{answered}) | {pct(answered, n)} | {pct(t['false_action'], n)} "
            f"({t['false_action']}) | {pct(abst, n)} ({t['abstained_right']} right) | "
            f"{pct(t['correct'], n)} | {p50 if p50 is not None else '–'} | "
            f"{p95 if p95 is not None else '–'} | "
            f"{(t['requests'] / n) if n else 0:.2f} |")


HEAD = ("| verb | arm | answering model | n | accuracy on answered | coverage | false actions | "
        "abstained | right of all | p50 ms | p95 ms | requests per case |\n"
        "|:--|:--|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|")


def reachable(set_name, case, gold):
    """Whether a concrete gold is among the options a cut thin input kept; None for open gold."""
    if not isinstance(gold, (list, dict)) and gold in validation_gold.OPEN_GOLD:
        return None
    root = SETS[set_name][0]
    v = case["verb"]
    if v == "why":
        _, back, _ = prepare((root / case["stdin"]).read_text().splitlines(), tail=True)
        return any(gold[0] <= b <= gold[1] for b in back)
    if v == "route":
        opts, _, _ = prepare(path_tools())
    else:
        kind = marker(case["argv"])[0] if v == "fill" else case["argv"][2]
        clone = REPOS[set_name] / case["env"]["repo"].split("/")[-1]
        opts, _, _ = prepare(listing(kind, clone))
        if kind == "commit":
            opts = [o.split()[0][:7] for o in opts]
    accepted = gold["any_of"] if isinstance(gold, dict) else [gold]
    return any(norm(str(a)) in {norm(o) for o in opts} for a in accepted)


def mechanism(case, j, th, jo, to, reach):
    """The jevify mechanism the difference on one item traces to, and a note for the item."""
    v = case["verb"]
    listed = "env" in case or (v == "route" and "inventory" not in case)
    note = None
    cut_off = reach is not None and not reach
    if (th.get("options_full") or 0) > MAX_OPTS:
        note = f"{th['options']}/{th['options_full']}" + (", gold cut off" if cut_off else "")
    if (th.get("options_full") or 0) > MAX_OPTS and cut_off:
        if listed:
            return "listers + chunking (thin read a 100-line cut of the listing)", note
        return "chunking into windows (thin read the last 100 lines)", note
    if jo.startswith("abstained") and to == "correct":
        return "abstention (jevify declined an answer thin got right)", note
    if jo.startswith("abstained"):
        return "calibrated abstain", note
    if to.startswith("abstained"):
        return "thin abstained", note
    if listed:
        return "listers (the listing jevify builds)", note
    if v == "route":
        return "evidence (tool summaries beside the names)", note
    if (j.get("requests") or 0) > 1 and v == "why":
        return "two-round finalists with failure context", note
    if (j.get("requests") or 0) > 1:
        return "two-round finalists", note
    return "question framing (same options, one request)", note


def variance_table(d, backends):
    """Per verb and arm: questions, how many moved across reruns / orders, and how many of those
    moved to a different confident answer rather than to an abstention."""
    out = []
    for arm in ("repeat", "order"):
        out.append(f"\n**{'Rerun (5 cold runs)' if arm == 'repeat' else 'Order (8 orders)'}**\n")
        out.append("| verb | arm | questions | answer changed | changed to another confident "
                   "answer | right in every run |\n|:--|:--|--:|--:|--:|--:|")
        gold = variance_gold()
        for b, label in backends:
            rows = read_jsonl(Path(d) / f"{arm}-{b}.jsonl")
            per = defaultdict(lambda: defaultdict(list))
            for r in rows:
                per[r["verb"]][r["id"]].append(r)
            for verb in sorted(per):
                qs = per[verb]
                changed = confident = allright = 0
                for qid, rs in qs.items():
                    answers = [r["answer"] for r in rs if r["answer"] != "not_run"]
                    if len(set(answers)) > 1:
                        changed += 1
                        if len({a for a in answers if a not in ABSTAIN}) > 1:
                            confident += 1
                    if answers and all(variance_right(gold.get(qid), a) for a in answers):
                        allright += 1
                out.append(f"| {verb} | {label} | {len(qs)} | {changed} | {confident} | "
                           f"{allright} |")
    return out


def variance_gold():
    import variance_score
    return variance_score.load_gold()


def variance_right(g, a):
    import variance_score
    return g is not None and variance_score.verdict(g, a) == "right"


def commits_table(d):
    out = ["| half | arm | answering model | n | right | wrong | lying subject won | "
           "abstained | requests per case | median s |", "|:--|:--|:--|--:|--:|--:|--:|--:|--:|--:|"]
    for half in ("scratch", "ripgrep"):
        for b, label in (("classifier", "jevify keyless"), ("typesafe", "jevify TypeSafe"),
                         ("thin", "thin")):
            rows = read_jsonl(Path(d) / f"commits-{half}-{b}.jsonl")
            if not rows:
                continue
            right = sum(r["correct"] for r in rows)
            abst = sum(1 for r in rows if not r.get("answer"))
            wrong = len(rows) - right - abst
            liar = sum(r["chose_claimant"] for r in rows)
            model = rows[0].get("model") or MODEL.get(b, "?")
            req = sum(r.get("requests") or 0 for r in rows) / len(rows)
            med = statistics.median(r["seconds"] for r in rows)
            out.append(f"| {half} | {label} | {model} | {len(rows)} | {right} | {wrong} | {liar} "
                       f"| {abst} | {req:.1f} | {med:.2f} |")
    return out


MODEL = {}


def cmd_report(d):
    d = Path(d)
    stamp = json.loads((d / "stamp.json").read_text())
    MODEL.update(thin=stamp["thin_model"])
    golds = {s: SETS[s][1].load_gold() for s in SETS}
    cases = {s: manifest(s) for s in SETS}
    arms = {}
    for s in SETS:
        for b in ("classifier", "typesafe", "thin"):
            for r in read_jsonl(d / f"{s}-{b}.jsonl"):
                r["set"] = s
                r["arm"] = r.get("arm") or b
                r["verb"] = cases[s].get(r["id"], {}).get("verb", r.get("verb"))
                arms.setdefault(b, []).append(r)
    thresholds = {r["id"]: r.get("threshold") for r in arms.get("classifier", [])}
    arms["thin+threshold"] = with_threshold(arms.get("thin", []), thresholds)
    models = {"classifier": model_of(arms.get("classifier")), "typesafe":
              model_of(arms.get("typesafe")), "thin": stamp["thin_model"],
              "thin+threshold": stamp["thin_model"]}
    labels = {"classifier": "jevify keyless", "typesafe": "jevify TypeSafe", "thin": "thin",
              "thin+threshold": "thin+threshold"}
    order = ["classifier", "typesafe", "thin", "thin+threshold"]
    tallies = {}
    for a in order:
        tallies[a] = tally(arms.get(a, []), lambda r: r["set"], golds, cases)
    verbs = sorted({v for a in order for v in tallies[a][0]})

    o = []
    o.append("# Verb scorecard\n")
    o.append(f"Measured {stamp['date']} on {stamp['host']} with `{stamp['version']}` "
             f"(`~/.cargo/bin/jevify`, sha256 `{stamp['sha256'][:16]}`), every call with "
             f"`JEVIFY_NO_CACHE=1`. Keyless backend: classifier.dev, answering model "
             f"`{models['classifier']}`. TypeSafe backend: answering model "
             f"`{models['typesafe']}`. Thin arm: `scripts/thinjev/jev` at `{stamp['thin_sha']}`, "
             f"keyless, answering model `{stamp['thin_model']}`.\n")
    o.append("The thin arm is one keyless call per question with the raw candidate lines as "
             "options and the top option as its answer: no lister, no excerpt or patch, no "
             "second round, no threshold. Its input is cut the way a shell user cuts it past the "
             "wrapper's 100 options: `tail -n 100` of a log, `head -n 100` of any other list, "
             "lines clipped to 200 characters and kept once. A lister case gives it `git "
             "ls-files`, the directories in it, or `git log --format='%h %s'`; a `route` case "
             "gives it the tool names. `thin+threshold` abstains where the top probability is "
             "below the threshold jevify reports for the same case. The thin arm has no TypeSafe "
             "column: the wrapper is keyless.\n")
    o.append("Sets: `evals/validation` (153 cases, both splits) and `evals/holdout` (103 cases) "
             "against their gold; `evals/variance` (the holdout subset, 5 cold reruns and 8 "
             "orders); `evals/commit-subjects` (20 commit descriptions whose subject lies). "
             "A `filter` or `label` case is one record; jevify answers a run of records in "
             "one call, the thin arm asks once per record. Latency is wall time per call, "
             "process start included; requests per case are HTTP requests to the backend.\n")
    o.append("**Columns.** accuracy on answered: right over decided. coverage: decided over "
             "scored. false actions: a decision the gold calls wrong or says not to make, over "
             "scored. abstained: no decision, with how many of those the gold calls right. "
             "right of all: right decisions over scored.\n")

    o.append("## Per verb, both sets\n")
    o.append(HEAD)
    for v in verbs:
        for a in order:
            t, lat = tallies[a]
            if t.get(v, {}).get("n"):
                o.append(line(v, labels[a], models[a], t[v], lat[v]))
    for a in order:
        t, lat = tallies[a]
        total, lats = defaultdict(int), []
        for v in t:
            for k, x in t[v].items():
                total[k] += x
            lats += lat[v]
        if total["n"]:
            o.append(line("**all verbs**", labels[a], models[a], total, lats))

    o.append("\n## jevify minus thin, keyless\n")
    o.append("Same backend, same model, same items. Differences are in points (percentage "
             "of scored items); positive means jevify is higher.\n")
    o.append("| verb | n | right of all | coverage | false-action rate | vs thin+threshold: right "
             "of all | false-action rate |\n|:--|--:|--:|--:|--:|--:|--:|")
    for v in verbs:
        J, T, TT = (tallies[a][0].get(v) for a in ("classifier", "thin", "thin+threshold"))
        if not (J and T and J["n"] and T["n"]):
            continue

        def rate(t, key):
            return 100 * t[key] / t["n"]

        def cov(t):
            return 100 * (t["correct"] + t["false_action"]) / t["n"]
        o.append(f"| {v} | {J['n']} | {rate(J, 'correct') - rate(T, 'correct'):+.0f} | "
                 f"{cov(J) - cov(T):+.0f} | {rate(J, 'false_action') - rate(T, 'false_action'):+.0f}"
                 f" | {rate(J, 'correct') - rate(TT, 'correct'):+.0f} | "
                 f"{rate(J, 'false_action') - rate(TT, 'false_action'):+.0f} |")

    o.append("\n## Where the gap comes from\n")
    o.append("Every item where jevify keyless and thin reach a different outcome, grouped by "
             "the jevify mechanism the difference traces to. `+` is an item jevify gets right "
             "or correctly declines and thin does not; `-` the reverse; `~` both wrong in "
             "different ways. `(100/292)`: the thin arm's options, out of the input's 292 non-blank lines; `gold cut off`: the right answer was not among them.\n")
    jrows = {(r["set"], r["id"]): r for r in arms.get("classifier", [])}
    groups = defaultdict(lambda: defaultdict(list))
    for r in arms.get("thin", []):
        key = (r["set"], r["id"])
        j = jrows.get(key)
        g = golds[r["set"]].get(r["id"])
        if j is None or g is None:
            continue
        mod = SETS[r["set"]][1]
        jo, to = outcome(mod, j["decision"], g["gold"]), outcome(mod, r["decision"], g["gold"])
        if "not_run" in (jo, to) or jo == to:
            continue
        good = {"correct", "abstained_right"}
        sign = "+" if jo in good and to not in good else "-" if to in good and jo not in good else "~"
        case = cases[r["set"]][r["id"]]
        reach = (reachable(r["set"], case, g["gold"])
                 if (r.get("options_full") or 0) > MAX_OPTS else None)
        mech, note = mechanism(case, j, r, jo, to, reach)
        groups[r["verb"]][mech].append(f"{sign}{r['id']}" + (f" ({note})" if note else ""))
    for v in sorted(groups):
        o.append(f"**{v}**\n")
        for mech, items in sorted(groups[v].items(), key=lambda kv: -len(kv[1])):
            plus = sum(1 for i in items if i.startswith("+"))
            minus = sum(1 for i in items if i.startswith("-"))
            o.append(f"- {mech}: {plus} for jevify, {minus} for thin, {len(items) - plus - minus} "
                     "both wrong — "
                     + ", ".join(f"`{i}`" for i in sorted(items)))
        o.append("")

    o.append("## Per set\n")
    o.append(HEAD)
    for s in SETS:
        for a in order:
            t, lat = tally([r for r in arms.get(a, []) if r["set"] == s], lambda r: r["set"],
                           golds, cases)
            for v in sorted(t):
                if t[v]["n"]:
                    o.append(line(f"{v} ({s})", labels[a], models[a], t[v], lat[v]))

    o.append("\n## Commits whose subject lies (`fill` commit kind)\n")
    o.append("`evals/commit-subjects`: the scratch repository (21 commits, 10 cases) and "
             "ripgrep at `3fce3b5` (2,287 commits, 10 cases). Right means the code commit "
             "that holds the change; the thin arm reads `git log --format='%h %s'`, the newest "
             "100 lines on ripgrep.\n")
    o.extend(commits_table(d))

    o.append("\n## Rerun and order sensitivity\n")
    o.append("`evals/variance` subset of the holdout set. A question has changed when its runs "
             "do not all give the same answer; a change to another confident answer is the "
             "kind a caller cannot see. Order applies to candidates that arrive as a list, so "
             "`why` has no order row.\n")
    o.extend(variance_table(d, [("classifier", "jevify keyless"), ("typesafe", "jevify TypeSafe"),
                                ("thin", "thin")]))

    t = spent(d)
    o.append(f"\n## Cost\n\nClassifications: {t['classifier']} keyless (jevify and "
             f"thin together), {t['typesafe']} on TypeSafe. Not run: "
             + (", ".join(f"{labels.get(a, a)} {sum(1 for r in arms.get(a, []) if r['decision'] == 'not_run')}"
                          for a in order) or "none") + ".\n")
    o.append("Regenerate: `bash evals/scorecard/run.sh` (TypeSafe arms run when "
             "`TYPESAFE_API_KEY_FILE` is set in its environment).")
    print("\n".join(o))


def model_of(rows):
    ms = sorted({r.get("model") for r in rows or [] if r.get("model") not in (None, "unknown")})
    return ", ".join(ms) or "–"


def main(argv):
    cmd, *rest = argv
    if cmd == "thin":
        cmd_thin(*rest)
    elif cmd == "thin-variance":
        cmd_thin_variance(*rest)
    elif cmd == "thin-commits":
        cmd_thin_commits(rest[0], Path(rest[1]), rest[2])
    elif cmd == "model":
        cmd_model()
    elif cmd == "spent":
        return cmd_spent(*rest)
    elif cmd == "report":
        cmd_report(rest[0])
    else:
        sys.exit(__doc__)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
