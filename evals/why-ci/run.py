#!/usr/bin/env python3
"""Fetch pinned CI logs and measure diagnostic selection, without dependencies."""

import argparse
import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time


def digest(data):
    return hashlib.sha256(data).hexdigest()


def percentile(values, fraction):
    return sorted(values)[max(0, math.ceil(len(values) * fraction) - 1)]


def execute(argv, raw, timeout=180):
    start = time.perf_counter()
    try:
        p = subprocess.run(argv, input=raw, capture_output=True, timeout=timeout)
        return p.returncode, p.stdout, time.perf_counter() - start
    except subprocess.TimeoutExpired:
        return 124, b"", time.perf_counter() - start


def measure(case, raw, binary, output):
    gold = set(case["gold_lines"])
    rows = []
    for name, args in [("why", []), ("why -n 3", ["-n", "3"])]:
        code, stdout, seconds = execute(
            [binary, "why", *args, "--json", "--no-cache"], raw
        )
        (output / f'{case["id"]}-{len(args)}.json').write_bytes(stdout)
        try:
            envelope = json.loads(stdout)
            causes = (envelope.get("data") or {}).get("causes", [])
            lines = [c["line"] for c in causes]
            valid = code in (0, 3) and envelope.get("exit_code") == code
            valid = valid and (bool(lines) if code == 0 else not lines)
            valid = valid and envelope.get("meta", {}).get("backend") == "typesafe"
            valid = valid and all(type(n) is int and 1 <= n <= case["line_count"] for n in lines)
        except (ValueError, KeyError, TypeError, AttributeError):
            valid, lines = False, []
        rows.append(dict(method=name, status="RUN" if valid else "NOT RUN",
                         exit_code=code, lines=lines, seconds=seconds,
                         tokens_out=len(stdout) / 4, abstain=valid and code == 3))
    start = time.perf_counter()
    p = subprocess.run(["tail", "-n", "50"], input=raw, capture_output=True, check=True)
    count = len(raw.splitlines())
    rows.append(dict(method="tail -n 50", status="RUN",
                     lines=list(range(max(1, count - 49), count + 1)),
                     tokens_out=len(p.stdout) / 4, seconds=time.perf_counter() - start,
                     abstain=False))
    start = time.perf_counter()
    p = subprocess.run(["grep", "-n", "-iE", "error|fail|panic"],
                       input=raw, capture_output=True)
    if p.returncode not in (0, 1):
        raise RuntimeError("grep failed")
    p = subprocess.run(["tail", "-n", "5"], input=p.stdout,
                       capture_output=True, check=True)
    lines = [int(line.split(b":", 1)[0]) for line in p.stdout.splitlines()]
    rows.append(dict(method="grep | tail -n 5", status="RUN", lines=lines,
                     tokens_out=len(p.stdout) / 4, seconds=time.perf_counter() - start,
                     abstain=not lines))
    for row in rows:
        row.update(id=case["id"], repo=case["repo"], ecosystem=case["ecosystem"],
                   ambiguous=case["ambiguous"], tokens_in=len(raw) / 4)
        row["hit1"] = bool(gold.intersection(row["lines"][:1]))
        row["hit3"] = bool(gold.intersection(row["lines"][:3]))
        row["coverage"] = bool(gold.intersection(row["lines"]))
        row["wrong"] = bool(row["lines"]) and not row["hit1"]
    return rows


def table(rows):
    print("| Method | N | hit@1 | hit@3 | Window hit | Abstain | Wrong first line | Tokens in → out | Fewer tokens | Wall p50 / p95 (s) |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for method in dict.fromkeys(r["method"] for r in rows):
        selected = [r for r in rows if r["method"] == method and r["status"] == "RUN"]
        label = method.replace("|", "\\|")
        n = len(selected)
        if not n:
            print(f"| {label} | 0 | NOT RUN | | | | | | | |")
            continue
        counts = [sum(r[key] for r in selected) for key in ("hit1", "hit3", "coverage", "abstain", "wrong")]
        rates = " | ".join(f"{k}/{n} ({100*k/n:.1f}%)" for k in counts)
        ti, to = (sum(r[key] for r in selected) for key in ("tokens_in", "tokens_out"))
        wall = [r["seconds"] for r in selected]
        print(f"| {label} | {n} | {rates} | {ti:.0f} → {to:.0f} | {100*(1-to/ti):.2f}% | {statistics.median(wall):.3f} / {percentile(wall, .95):.3f} |")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="jevify")
    parser.add_argument("--cases", type=Path, default=Path(__file__).with_name("cases.jsonl"))
    parser.add_argument("--cache", type=Path, default=Path(os.environ.get("JEVIFY_WHY_CI_CACHE", "~/.cache/jevify-why-ci")).expanduser())
    args = parser.parse_args()
    if not os.environ.get("TYPESAFE_API_KEY_FILE") and not os.environ.get("TYPESAFE_API_KEY"):
        parser.error("set TYPESAFE_API_KEY_FILE or TYPESAFE_API_KEY for the TypeSafe backend")
    repo = Path(__file__).resolve().parents[2]
    if args.cache.resolve().is_relative_to(repo):
        parser.error("raw-log cache must be outside the repository")
    cases = [json.loads(line) for line in args.cases.read_text().splitlines() if line.strip()]
    if len(cases) > 60 or len({c["id"] for c in cases}) != len(cases):
        parser.error("require unique cases and at most 120 jevify calls (60 cases)")
    args.cache.mkdir(parents=True, exist_ok=True)
    output = args.cache / dt.datetime.now(dt.timezone.utc).strftime("results-%Y%m%dT%H%M%S.%fZ")
    output.mkdir()
    version = subprocess.run([args.binary, "--version"], capture_output=True, check=True).stdout.decode().strip()
    (output / "measurement.json").write_text(json.dumps(dict(
        version=version, backend="typesafe", measured_at=dt.datetime.now(dt.timezone.utc).isoformat(),
        cases_sha256=digest(args.cases.read_bytes()), planned_calls=2 * sum(not c["ambiguous"] for c in cases)
    ), indent=2))
    os.environ["JEVIFY_BACKEND"] = "typesafe"
    rows, skipped = [], []
    for case in cases:
        path = args.cache / f'{case["id"]}.log'
        if not path.exists():
            p = subprocess.run(["gh", "run", "view", str(case["id"]), "--log-failed", "-R", case["repo"]], capture_output=True)
            if p.returncode or not p.stdout:
                skipped.append(dict(id=case["id"], reason="log unavailable or expired"))
                continue
            path.write_bytes(p.stdout)
        raw = path.read_bytes()
        if digest(raw) != case["sha256"] or len(raw.splitlines()) != case["line_count"]:
            skipped.append(dict(id=case["id"], reason="log hash or line count mismatch"))
            continue
        if case["ambiguous"]:
            skipped.append(dict(id=case["id"], reason="ambiguous gold; excluded before inference"))
            continue
        print(f'Running {case["repo"]} {case["id"]}', file=sys.stderr, flush=True)
        rows.extend(measure(case, raw, args.binary, output))
        (output / "results.json").write_text(json.dumps(dict(rows=rows, skipped=skipped), indent=2))
    (output / "results.json").write_text(json.dumps(dict(rows=rows, skipped=skipped), indent=2))
    table(rows)
    print("\nPer-ecosystem top-1 results:\n")
    print("| Ecosystem | N | Hit | Abstain | Wrong |")
    print("|---|---:|---:|---:|---:|")
    for eco in sorted({c["ecosystem"] for c in cases}):
        group = [r for r in rows if r["method"] == "why" and r["ecosystem"] == eco and r["status"] == "RUN"]
        print(f'| {eco} | {len(group)} | {sum(r["hit1"] for r in group)} | {sum(r["abstain"] for r in group)} | {sum(r["wrong"] for r in group)} |')
    for item in skipped:
        print(f'NOT RUN {item["id"]}: {item["reason"]}')
    failed = [r for r in rows if r["status"] != "RUN"]
    for row in failed:
        print(f'NOT RUN {row["id"]} {row["method"]}: exit {row["exit_code"]}')
    print(f"\nPrivate results: {output}", file=sys.stderr)
    return 1 if skipped or failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
