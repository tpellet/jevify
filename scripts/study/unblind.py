#!/usr/bin/env python3
"""Join the blind scores to the arms and report each cell with an interval.

    python3 scripts/study/unblind.py

Runs after score.py. A cell is one task in one arm; its correctness is k of n
with a Wilson score interval at 95 %, never a bare count. At n = 3 that
interval runs from about 0.29 to 1.00 for 3 of 3, which is the point: the
interval says what the count cannot.
"""

import collections
import json
import math
import os
import pathlib

def _study_dir():
    """$JEVSTUDY decides which binary and which repositories this touches, so it is
    resolved and required to be an existing directory before anything uses it."""
    p = pathlib.Path(os.environ.get("JEVSTUDY") or (pathlib.Path.home() / "jevify-study")).resolve()
    if not p.is_dir():
        raise SystemExit(f"JEVSTUDY is not a directory: {p}")
    return p


STUDY = _study_dir()


def wilson(k, n, z=1.96):
    if n == 0:
        return (0.0, 1.0)
    p = k / n
    d = 1 + z * z / n
    c = p + z * z / (2 * n)
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n))
    return ((c - h) / d, (c + h) / d)


def main():
    m = json.loads((STUDY / "unblind_map.json").read_text())
    scored = [json.loads(l) for l in (STUDY / "scored.jsonl").read_text().splitlines() if l.strip()]
    metas = {}
    for run in sorted((STUDY / "runs").iterdir()):
        f = run / "meta.json"
        if f.exists():
            meta = json.loads(f.read_text())
            metas[meta["rid"]] = meta

    cells = collections.defaultdict(list)
    print(f"{'rid':22s} {'status':13s} {'ok':3s} {'turns':>5s} {'tools':>5s} {'in_tok':>8s} "
          f"{'wall_s':>7s} {'agent$':>8s} {'jev':>4s} {'req':>4s} {'esc':>4s}")
    for r in sorted(scored, key=lambda r: m[r["bid"]]["rid"]):
        info = m[r["bid"]]
        meta = metas.get(info["rid"], {})
        cells[(info["task"], info["arm"])].append(r)
        print(f"{info['rid']:22s} {r['status']:13s} {str(r['correct']):3s} "
              f"{str(meta.get('turns')):>5s} {str(meta.get('tool_uses')):>5s} "
              f"{str(meta.get('input_tokens')):>8s} {str(meta.get('wall_s')):>7s} "
              f"{meta.get('agent_cost_usd') or 0:8.4f} {str(meta.get('jevify_calls')):>4s} "
              f"{str(meta.get('jevify_requests')):>4s} {str(len(meta.get('escapes') or [])):>4s}")

    print(f"\n{'cell':16s} {'k/n':>7s}  95% Wilson       refused")
    for (task, arm), rs in sorted(cells.items()):
        usable = [r for r in rs if r["status"] != "refused_leak"]
        k = sum(r["correct"] or 0 for r in usable)
        n = len(usable)
        lo, hi = wilson(k, n)
        print(f"{task + ' ' + arm:16s} {k:3d}/{n:<3d}  [{lo:.2f}, {hi:.2f}]   "
              f"{sum(1 for r in rs if r['status'] == 'refused_leak')}")

    print("\nresources per arm (sum over the runs above)")
    for arm in ("with", "without"):
        rids = [i["rid"] for i in m.values() if i["arm"] == arm]
        ms = [metas[r] for r in rids if r in metas]
        if not ms:
            continue
        print(f"  {arm:8s} runs {len(ms):2d}  turns {sum(x.get('turns') or 0 for x in ms):4d}"
              f"  tools {sum(x.get('tool_uses') or 0 for x in ms):4d}"
              f"  input_tokens {sum(x.get('input_tokens') or 0 for x in ms):9,d}"
              f"  wall {sum(x.get('wall_s') or 0 for x in ms):7.1f}s"
              f"  agent ${sum(x.get('agent_cost_usd') or 0 for x in ms):.4f}"
              f"  jevify calls {sum(x.get('jevify_calls') or 0 for x in ms):3d}"
              f"  requests {sum(x.get('jevify_requests') or 0 for x in ms):3d}"
              f"  questions {sum(x.get('jevify_questions') or 0 for x in ms):4d}")

    esc = [(r, metas[r].get("escapes")) for r in metas if metas[r].get("escapes")]
    print(f"\nruns that wrote outside their directory: {len(esc)}")
    for rid, files in esc:
        print(f"  {rid}: {files}")


if __name__ == "__main__":
    main()
