#!/usr/bin/env python3
"""Summarize TSRS_RESOLVER_FS_STATS JSON; optionally compare two runs.

    python3 tools/perf/resolver-fs.py before.json [after.json] [--paths]

Prints JSON aggregates without paths by default. --paths includes the busiest missing-parent directories and
repeated metadata paths; use only for reports whose paths may be disclosed. Comparison exits 1 if distinct
logical (site, operation, path, result) tuples change. Concurrent call-count differences are reported separately.
These are instrumented API calls, not syscalls: directory iteration may issue none, while api.read hides several.
Cache paths identify the enclosing logical request, not the actual interned key; nonresolver cache paths are empty.
Optional physical_events add ordered candidate-overlap counts. Observed API completion does not establish cache
publication or redundancy; metadata may require information that a directory entry or successful read lacks.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path, PurePosixPath


METADATA = {"api.metadata", "api.symlink_metadata", "api.metadata.case_probe", "api.metadata.proc_probe",
            "rustix.statx.link", "rustix.statx.follow"}
LOGICAL_KEY = ("site", "operation", "path", "result")
EVENT_FIELDS = ("thread", "site", "logical_operation", "logical_path", "operation", "path")
ORDERED_METADATA = METADATA | {"api.symlink_metadata.reparse"}
FILE_TYPES = {"file", "directory", "symlink", "special"}  # osvfs/os.rs::file_type_result labels


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
    if "physical_events" in document and not isinstance(document["physical_events"], list):
        raise ValueError("report physical_events must be a list")
    return document["rows"], document.get("physical_events")


def counts(rows, fields):
    result = Counter()
    for row in rows:
        result[tuple(row[field] for field in fields)] += row["count"]
    return result


def table(values, fields):
    return [{**dict(zip(fields, key)), "count": count} for key, count in
            sorted(values.items(), key=lambda item: (-item[1], item[0]))]


def normalized_absolute_posix(path):
    return path == "/" or (path.startswith("/") and "\\" not in path
                           and all(part not in ("", ".", "..") for part in path[1:].split("/")))


def ordered_summary(events, paths):
    pending, active, completed = {}, Counter(), Counter()
    started_operations, completed_results, candidates, candidate_paths = Counter(), Counter(), Counter(), Counter()
    dirents, reads, missing = {}, set(), set()
    listings, listed_directories = {}, set()
    completed_listings = 0
    after_dirent = Counter()
    for sequence, event in enumerate(events, 1):
        if not isinstance(event, dict) or type(event.get("sequence")) is not int or event["sequence"] != sequence:
            raise ValueError(f"physical event {sequence}: expected contiguous 1-based sequence")
        if type(event.get("call")) is not int or event["call"] < 1:
            raise ValueError(f"physical event {sequence}: call must be a positive integer")
        if any(not isinstance(event.get(field), str) for field in EVENT_FIELDS if field != "logical_path"):
            raise ValueError(f"physical event {sequence}: missing or non-string context fields")
        if "logical_path" not in event or (event["logical_path"] is not None and not isinstance(event["logical_path"], str)):
            raise ValueError(f"physical event {sequence}: logical_path must be a string or null")
        operation, path, call = event["operation"], event["path"], event["call"]
        key = (operation, path)
        if event.get("phase") == "start":
            if call != sequence or "result" not in event or event["result"] is not None:
                raise ValueError(f"physical event {sequence}: start requires call == sequence and null result")
            overlaps = []
            if operation != "api.ReadDir.next":
                if active[key]:
                    overlaps.append("inflight_exact_duplicate")
                if completed[key]:
                    overlaps.append("completed_exact_duplicate")
            if normalized_absolute_posix(path):
                if any(str(parent) in missing for parent in PurePosixPath(path).parents):
                    overlaps.append("known_missing_ancestor")
                if operation in ORDERED_METADATA and path != "/" and str(PurePosixPath(path).parent) in listed_directories:
                    overlaps.append("metadata_after_parent_listing")
            if operation in {"api.read_dir", "rustix.statfs"} and path in listed_directories:
                overlaps.append("directory_probe_after_listing")
            dtype = dirents.get(path) if operation in ORDERED_METADATA else None
            if operation in ORDERED_METADATA and path in reads:
                overlaps.append("metadata_after_successful_read")
            pending[call] = (event, overlaps, dtype)
            active[key] += 1
            started_operations[(operation,)] += 1
            continue
        if event.get("phase") != "end" or not isinstance(event.get("result"), str):
            raise ValueError(f"physical event {sequence}: expected start or end with a string result")
        if call not in pending:
            raise ValueError(f"physical event {sequence}: end has no unmatched start")
        start, overlaps, dtype = pending.pop(call)
        if any(start[field] != event[field] for field in EVENT_FIELDS):
            raise ValueError(f"physical event {sequence}: end context differs from start")
        active[key] -= 1
        completed[key] += 1
        result = event["result"]
        attribution = "nonresolver" if event["site"] == "nonresolver" else "resolver"
        completed_results[(operation, result)] += 1
        if "metadata_after_parent_listing" in overlaps and result == "error.not_found":
            overlaps.append("missing_metadata_after_parent_listing")
        for kind in overlaps:
            candidates[(kind, operation, attribution)] += 1
            if paths:
                candidate_paths[(kind, operation, attribution, path)] += 1
        if dtype is not None:
            after_dirent[(dtype, operation, result, attribution)] += 1
            if paths:
                candidate_paths[("metadata_after_dirent", operation, attribution, path)] += 1
        # Only completed observations can provide evidence for a later call. Successful observations
        # clear a previous missing fact; none of these observations proves that the filesystem is stable.
        if operation == "api.DirEntry.file_type" and result in FILE_TYPES:
            dirents[path] = result
            missing.discard(path)
        if operation == "api.read" and result == "ok":
            reads.add(path)
            missing.discard(path)
        if operation in ORDERED_METADATA:
            if result == "error.not_found" and normalized_absolute_posix(path):
                missing.add(path)
            elif not result.startswith("error."):
                missing.discard(path)
        # The trace has no iterator handle or raw entry names. A successful open followed by an
        # error-free end on the same thread/path establishes only a candidate ceiling for reuse.
        listing_key = (event["thread"], path)
        if operation == "api.read_dir":
            listings[listing_key] = result == "ok"
        elif operation == "api.ReadDir.next":
            if result == "end":
                if listings.pop(listing_key, False):
                    listed_directories.add(path)
                    completed_listings += 1
            elif result != "ok":
                listings[listing_key] = False
    result = {
        "events": len(events), "started_calls": sum(started_operations.values()),
        "completed_calls": sum(completed_results.values()), "unmatched_starts": len(pending),
        "validation": "ordered sequences and paired call/context fields validated; unmatched starts may unwind or remain active",
        "candidate_unit": "completed calls with prior API observations; categories overlap and do not prove redundant work",
        "ordering_caveat": "API completion precedes cache publication; completed duplicates can still be racing calls",
        "ancestor_scope": "absolute normalized POSIX paths only: no dot components, backslashes, doubled or trailing separators",
        "listing_scope": "same thread/path read_dir ok followed by ReadDir.next end without iteration errors; no raw-name proof",
        "listing_candidates": "upper bounds, including missing results; directory enumeration need not expose every lookup-visible name",
        "completed_directory_listings": completed_listings, "completed_listing_directories": len(listed_directories),
        "started_operations": table(started_operations, ("operation",)),
        "completed_operations_results": table(completed_results, ("operation", "result")),
        "candidate_overlaps": table(candidates, ("candidate", "operation", "attribution")),
        "metadata_after_dirent": table(after_dirent, ("dirent_type", "operation", "result", "attribution")),
    }
    if paths:
        result["candidate_paths"] = table(candidate_paths, ("candidate", "operation", "attribution", "path"))[:20]
        result["unmatched_start_details"] = [start for start, _, _ in pending.values()][:20]
    return result


def summarize(rows, paths, events=None):
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
    if events is not None:
        result["ordered_physical"] = ordered_summary(events, paths)
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
        before, before_events = load(args.before)
        result = {"physical_unit": "instrumented API calls, not kernel syscalls",
                  "metadata_unit": "per-path stat/lstat, including startup case/procfs probes; excludes statfs",
                  "before": summarize(before, args.paths, before_events)}
        if args.after:
            after, after_events = load(args.after)
            result["after"] = summarize(after, args.paths, after_events)
            result["comparison"] = compare(before, after, args.paths)
    except (OSError, ValueError, TypeError) as error:
        parser.error(str(error))
    print(json.dumps(result, indent=2))
    return int(not result.get("comparison", {}).get("logical_results_identical", True))


if __name__ == "__main__":
    raise SystemExit(main())
