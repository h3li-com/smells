#!/usr/bin/env python3
"""Summarize cargo-mutants outcomes without hiding non-tested statuses."""

from __future__ import annotations

import json
import sys
from collections import Counter
from pathlib import Path


def summarize(path: Path) -> dict[str, object]:
    document = json.loads(path.read_text(encoding="utf-8"))
    mutant_outcomes = [
        outcome
        for outcome in document["outcomes"]
        if isinstance(outcome.get("scenario"), dict)
        and "Mutant" in outcome["scenario"]
    ]
    statuses = Counter(outcome["summary"] for outcome in mutant_outcomes)
    caught = statuses["CaughtMutant"]
    survived = statuses["MissedMutant"]
    tested = caught + survived
    total = len(mutant_outcomes)
    return {
        "total_mutants": total,
        "caught": caught,
        "survived": survived,
        "primary_score_percent": round(100 * caught / tested, 2) if tested else None,
        "conservative_score_percent": round(100 * caught / total, 2) if total else None,
        "status_counts": dict(sorted(statuses.items())),
    }


def main(arguments: list[str]) -> int:
    if len(arguments) != 2:
        print("usage: mutation-summary.py OUTCOMES_JSON", file=sys.stderr)
        return 2
    try:
        summary = summarize(Path(arguments[1]))
    except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        print(f"cannot summarize mutation outcomes: {error}", file=sys.stderr)
        return 2
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
