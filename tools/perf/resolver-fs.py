#!/usr/bin/env python3
"""Summarize TSRS_RESOLVER_FS_STATS JSON; optionally compare two runs.

    python3 tools/perf/resolver-fs.py before.json [after.json] [--paths]

Prints JSON aggregates without paths by default. --paths includes the busiest missing-parent directories and
repeated metadata paths; use only for reports whose paths may be disclosed. Comparison exits 1 if distinct
logical (site, operation, path, result) tuples change. Concurrent call-count differences are reported separately.
These are instrumented API calls, not syscalls: directory iteration may issue none, while api.read hides several.
Cache paths identify the enclosing logical request, not the actual interned key; nonresolver cache paths are empty.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path, PurePosixPath


METADATA = {"api.metadata", "api.symlink_metadata", "api.metadata.case_probe", "api.metadata.proc_probe",
            "rustix.statx.link", "rustix.statx.follow"}
LOGICAL_KEY = ("site", "operation", "path", "result")


def load(path):
    document = json.loads(path.read_text())
    if not isinstance(document, dict) or document.get("version") != 1 or not isinstance(document.get("rows"), list):
        raise ValueError("expected a version 1 resolver filesystem report")
    for row in document["rows"]:
        if not isinstance(row, dict) or any(not isinstance(row.get(key), str) for key in
                                           ("layer", "site", "logical_operation", "operation", "path", "result")):
            raise ValueError("report row has missing or non-string fields")
        if type(row.get("count")) is not int or row["count"] < 1:
            raise ValueError("report row count must be a positive integer")
    return document["rows"]


def counts(rows, fields):
    result = Counter()
    for row in rows:
        result[tuple(row[field] for field in fields)] += row["count"]
    return result


def table(values, fields):
    return [{**dict(zip(fields, key)), "count": count} for key, count in
            sorted(values.items(), key=lambda item: (-item[1], item[0]))]


def summarize(rows, paths):
    physical = [row for row in rows if row["layer"] == "physical"]
    metadata = [row for row in physical if row["operation"] in METADATA]
    repeated = {key: count for key, count in counts(metadata, ("operation", "path")).items() if count > 1}
    missing = [row for row in metadata if row["result"] == "error.not_found"]
    parents = defaultdict(list)
    for row in missing:
        parents[str(PurePosixPath(row["path"]).parent)].append(row)
    parent_counts = sorted(((parent, sum(row["count"] for row in group)) for parent, group in parents.items()),
                           key=lambda item: (-item[1], item[0]))
    attribution = [{**row, "attribution": "nonresolver" if row["site"] == "nonresolver" else "resolver"}
                   for row in physical]
    result = {
        "physical_operations_results": table(counts(physical, ("operation", "result")), ("operation", "result")),
        "physical_attribution": table(counts(attribution, ("attribution", "operation")), ("attribution", "operation")),
        "physical_sites": table(counts(physical, ("site", "operation")), ("site", "operation")),
        "logical_operations": table(counts([row for row in rows if row["layer"] == "logical"],
                                            ("site", "operation", "result")), ("site", "operation", "result")),
        "cache_outcomes": table(counts([row for row in rows if row["layer"] == "cache"],
                                      ("operation", "result")), ("operation", "result")),
        "metadata_calls": sum(row["count"] for row in metadata),
        "resolver_metadata_calls": sum(row["count"] for row in metadata if row["site"] != "nonresolver"),
        "repeated_metadata_operation_paths": len(repeated),
        "repeated_metadata_calls": sum(count - 1 for count in repeated.values()),
        "missing_metadata_calls": sum(row["count"] for row in missing),
        "missing_candidate_paths": len({row["path"] for row in missing}),
        "missing_parent_directories": len(parents),
        "busiest_missing_parent_counts": [count for _, count in parent_counts[:20]],
    }
    if paths:
        result["repeated_metadata_paths"] = table(repeated, ("operation", "path"))[:20]
        result["busiest_missing_parents"] = [
            {"parent": parent, "count": count, "unique_candidates": len({row["path"] for row in parents[parent]}),
             "sites": table(counts(parents[parent], ("site",)), ("site",))}
            for parent, count in parent_counts[:20]
        ]
    return result


def compare(before, after, paths):
    logical = [counts([row for row in rows if row["layer"] == "logical"], LOGICAL_KEY) for rows in (before, after)]
    added, removed = logical[1].keys() - logical[0].keys(), logical[0].keys() - logical[1].keys()
    result = {"logical_results_identical": not added and not removed, "logical_call_counts_identical": logical[0] == logical[1],
              "logical_distinct_added": len(added), "logical_distinct_removed": len(removed)}
    if paths:
        result["logical_added"] = [dict(zip(LOGICAL_KEY, key)) for key in sorted(added)[:20]]
        result["logical_removed"] = [dict(zip(LOGICAL_KEY, key)) for key in sorted(removed)[:20]]
    fields = ("layer", "operation", "result")
    left, right = counts(before, fields), counts(after, fields)
    changes = []
    for key in left.keys() | right.keys():
        a, b = left[key], right[key]
        changes.append({**dict(zip(fields, key)), "before": a, "after": b, "delta": b - a,
                        "change_percent": (b / a - 1) * 100 if a else None})
    result["operation_changes"] = sorted(changes, key=lambda row: (-abs(row["delta"]), row["layer"], row["operation"], row["result"]))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", nargs="?", type=Path)
    parser.add_argument("--paths", action="store_true", help="include paths in top lists and logical differences")
    args = parser.parse_args()
    try:
        before = load(args.before)
        result = {"physical_unit": "instrumented API calls, not kernel syscalls",
                  "metadata_unit": "per-path stat/lstat, including startup case/procfs probes; excludes statfs",
                  "before": summarize(before, args.paths)}
        if args.after:
            after = load(args.after)
            result["after"] = summarize(after, args.paths)
            result["comparison"] = compare(before, after, args.paths)
    except (OSError, ValueError, TypeError) as error:
        parser.error(str(error))
    print(json.dumps(result, indent=2))
    return int(not result.get("comparison", {}).get("logical_results_identical", True))


if __name__ == "__main__":
    raise SystemExit(main())
