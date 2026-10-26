#!/usr/bin/env python3
"""Measure fix-message concentration on prepared pinned showcase graphs and full histories."""
import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import datetime
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def git_data(repo, arguments, timeout, input=None):
    process = subprocess.Popen(["git", "-C", repo, *arguments], stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
    try:
        stdout, stderr = process.communicate(input, timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate()
        raise TimeoutError("Git data preparation timed out")
    if process.returncode:
        raise ValueError(stderr[-2000:])
    return stdout


def prefetch(entry, timeout):
    """Batch missing partial-clone objects; never modify the checkout or Git refs."""
    start = time.perf_counter()
    remaining = lambda: max(.1, timeout - (time.perf_counter() - start))
    repo, commit = entry["repo_path"], entry["commit"]
    timestamp = int(git_data(repo, ["show", "-s", "--format=%ct", commit], remaining()).strip())
    commits = git_data(repo, ["rev-list", f"--since-as-filter=@{timestamp-365*86400}", "--boundary", commit], remaining())
    commits = "\n".join(line.removeprefix("-") for line in commits.splitlines()) + "\n"
    objects = git_data(repo, ["rev-list", "--objects", "--no-walk", "--missing=print", "--stdin"], remaining(), commits)
    missing = [line[1:] for line in objects.splitlines() if line.startswith("?")]
    for oid in missing:
        if len(oid) not in (40, 64) or any(c not in "0123456789abcdef" for c in oid):
            raise ValueError("invalid missing-object ID from Git")
    for offset in range(0, len(missing), 4000):
        if time.perf_counter() - start >= timeout:
            raise TimeoutError("Git object preparation exceeded its budget")
        git_data(repo, ["-c", "fetch.negotiationAlgorithm=noop", "fetch", "--quiet", "--no-tags", "--no-write-fetch-head",
                       "--no-auto-maintenance", "--recurse-submodules=no", "--stdin", "origin"], remaining(),
                 "\n".join(missing[offset:offset+4000]) + "\n")
    return {"missing_objects": len(missing), "seconds": time.perf_counter() - start}


def aggregate(rows):
    valid = [r for r in rows if r.get("status") == "ok" and r["validation"]["bugfix_commits"] > 0]
    fixes = sum(r["validation"]["bugfix_commits"] for r in valid)
    top = sum(r["validation"]["top_bugfix_commits"] for r in valid)
    expected = sum(r["validation"]["bugfix_commits"] * r["validation"].get("expected_random_share", 0) for r in valid)
    return {"attempted": len(rows), "completed": sum(r.get("status") == "ok" for r in rows),
            "with_fix_commits": len(valid), "bugfix_commits": fixes, "top_bugfix_commits": top,
            "share": top / fixes if fixes else None,
            "expected_random_share": expected / fixes if fixes else None,
            "random_lift": top / expected if expected else None,
            "caveat": "In-sample association with fix/bug/regression commit messages, not verified defects or predictive validation. Random comparison controls each commit's number of touched scanned files."}


def measure(entry, binary, output, timeout, prepare=False):
    name = entry["name"]
    result = {"name": name, "repo": entry["repo"], "commit": entry["commit"]}
    path = output / f"{name}.json"
    error = output / f"{name}.log"
    started = time.perf_counter()
    command = [str(binary), "hotspots", entry["graph_path"], "--root", entry["scan_root"],
               "--since", "12 months ago", "--authors", "--format", "json", "--top", str(entry.get("files", 100000))]
    try:
        if not entry.get("pin_matches") or entry.get("shallow"):
            raise ValueError("missing exact pin or full history")
        if prepare:
            result["preparation"] = prefetch(entry, timeout)
        with path.open("w") as stdout, error.open("w") as stderr:
            process = subprocess.Popen(command, stdout=stdout, stderr=stderr, start_new_session=True)
            try:
                code = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                raise TimeoutError(f"exceeded {timeout}s including lazy Git blob downloads")
        if code:
            raise ValueError(error.read_text()[-2000:])
        report = json.loads(path.read_text())
        result.update({"status": "ok", "commits": report["commits"], "skipped": report["skipped"],
                       "shallow": report["shallow"], "validation": report["validation"],
                       "limitations": report["limitations"]})
        result["raw_report_sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
        result["top_examples"] = [{k: row[k] for k in ("path", "commits", "authors", "churn", "coupling", "cycle", "score", "cycle_size", "cut_candidate")} for row in report["files"][:3]]
        result["graph_url"] = entry.get("graph_url")
        result["scan_root_path"] = entry.get("path", ".")
        result["collector_unresolved"] = entry.get("source", {}).get("collector", {}).get("unresolved")
    except (OSError, ValueError, TimeoutError, KeyError) as failure:
        result.update({"status": "failed", "error": str(failure)})
    result["seconds"] = time.perf_counter() - started
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--coverage", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parent.parent / "target/release/boundrail")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=240)
    parser.add_argument("--jobs", type=int, default=3)
    parser.add_argument("--prefetch", action="store_true", help="batch missing partial-clone objects before scoring")
    parser.add_argument("--only", action="append", default=[])
    args = parser.parse_args()
    if args.timeout < 1 or args.jobs < 1:
        parser.error("timeout and jobs must be positive")
    entries = json.loads(args.coverage.read_text())
    if set(args.only) - {e["name"] for e in entries}:
        parser.error("unknown repository in --only")
    entries = [e for e in entries if not args.only or e["name"] in args.only]
    args.out.mkdir(parents=True, exist_ok=True)
    rows = []
    metadata = {"measured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                "binary_sha256": hashlib.sha256(args.binary.resolve().read_bytes()).hexdigest(),
                "method": {"window": "365 days ending at each graph commit", "top_files": "ceil(10% of scanned files), ranked per repository",
                           "authors": True, "max_commit_files": 30, "merges": "excluded", "normalization": "per scanned root",
                           "random_baseline": "uniform sampling without replacement, controlling each fix commit's scanned file count",
                           "validation": "in-sample fix/bug/regression message association; no causal or predictive claim"}}
    with ThreadPoolExecutor(max_workers=args.jobs) as executor:
        futures = [executor.submit(measure, e, args.binary.resolve(), args.out, args.timeout, args.prefetch) for e in entries]
        for future in as_completed(futures):
            row = future.result()
            rows.append(row)
            print(json.dumps(row), flush=True)
            report = {**metadata, "summary": aggregate(rows), "repositories": sorted(rows, key=lambda r: r["name"])}
            (args.out / "validation.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if all(r["status"] == "ok" for r in rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
