#!/usr/bin/env python3
"""Pinned source-graph benchmarks: standard library only; never builds target projects."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import tempfile
import time


HERE = Path(__file__).resolve().parent
GRAPH_FIELDS = ("config", "files", "edges", "modules", "module_edges", "cycles", "summary", "findings")


def run(command, cwd=None):
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"{' '.join(map(str, command))}: {result.stderr.strip()}")
    return result.stdout.strip()


def check(case, document):
    """Independent source-reference assertions, not an exhaustive graph oracle."""
    errors = []
    history = document.get("kind") == "history"
    graphs = [s["graph"] for s in document["snapshots"]] if history else [document]
    counts = [len(g["files"]) for g in graphs]
    expected = case.get("files")
    if expected is not None and (counts if history else counts[0]) != expected:
        errors.append(f"file counts {counts}, expected {expected}")
    edges = {(e["from"], e["to"]) for e in graphs[-1]["edges"]}
    for pair in case.get("edges", []):
        if tuple(pair) not in edges:
            errors.append(f"missing source reference {pair}")
    if case.get("no_package_edges") and any(to.endswith("/package-info.java") for _, to in edges):
        errors.append("package names created false dependencies on package-info.java")
    if "unresolved" in case and graphs[-1]["source"]["collector"].get("unresolved", 0) != case["unresolved"]:
        errors.append("unexpected unresolved diagnostics")
    unchanged = case.get("unchanged", [])
    if unchanged:
        reference = {k: graphs[unchanged[0]][k] for k in GRAPH_FIELDS}
        for index in unchanged[1:]:
            if {k: graphs[index][k] for k in GRAPH_FIELDS} != reference:
                errors.append(f"source-free commit changed graph at snapshot {index}")
    if "removed_file" in case:
        removed = case["removed_file"]
        if removed not in {f["path"] for f in graphs[0]["files"]}:
            errors.append("deleted source missing from initial snapshot")
        if any(removed in {f["path"] for f in g["files"]} for g in graphs[1:]):
            errors.append("deleted source survived in later snapshots")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=HERE.parent / "target/release/archgraph")
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--only", action="append", default=[])
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    binary = args.binary.resolve(strict=True)
    work = args.work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    cases = json.loads((HERE / "repos.json").read_text())
    if set(args.only) - {c["name"] for c in cases}:
        parser.error("unknown benchmark in --only")
    results = []
    for case in cases:
        if args.only and case["name"] not in args.only:
            continue
        print(case["name"], flush=True)
        row = {"name": case["name"], "repo": case["repo"], "commit": case["commit"]}
        try:
            checkout = work / "repos" / case.get("checkout", case["name"])
            if not checkout.exists():
                checkout.parent.mkdir(parents=True, exist_ok=True)
                with tempfile.TemporaryDirectory(dir=checkout.parent, prefix=f"{checkout.name}-") as staging:
                    run(["git", "init", "-q", staging])
                    run(["git", "-C", staging, "remote", "add", "origin", case["repo"]])
                    run(["git", "-C", staging, "fetch", "-q", "--depth", "8", "origin", case["commit"]])
                    run(["git", "-C", staging, "-c", "core.hooksPath=/dev/null", "checkout", "-q", "--detach", "FETCH_HEAD"])
                    Path(staging).rename(checkout)
            if run(["git", "-C", str(checkout), "rev-parse", "HEAD"]) != case["commit"]:
                raise RuntimeError(f"{checkout} is not at the pinned commit")
            if run(["git", "-C", str(checkout), "-c", "core.fsmonitor=false", "status", "--porcelain"]):
                raise RuntimeError(f"{checkout} has uncommitted changes")
            dest = work / "outputs" / case["name"]
            dest.mkdir(parents=True, exist_ok=True)
            mode = case.get("mode", "scan")
            command = [str(binary), mode, str(checkout), "--language", case["language"], *case.get("flags", [])]
            command += ["--out-dir", str(dest)] if mode == "history" else ["-o", str(dest / "scan.json")]
            samples, errors = [], set()
            # One untimed warm-up, then sequential measured runs. Save the final raw artifacts.
            for index in range(args.runs + 1):
                start = time.perf_counter()
                result = subprocess.run(command, capture_output=True, text=True)
                seconds = time.perf_counter() - start
                (dest / "stderr.log").write_text(result.stderr)
                if result.returncode:
                    raise RuntimeError(result.stderr.strip())
                document = json.loads((dest / ("history.json" if mode == "history" else "scan.json")).read_text())
                errors.update(check(case, document))
                if index:
                    samples.append(seconds)
            graph = document["snapshots"][-1]["graph"] if mode == "history" else document
            row.update({"seconds": samples, "median_seconds": statistics.median(samples),
                        "files": len(graph["files"]), "edges": len(graph["edges"]),
                        "collector": {k: graph["source"]["collector"][k] for k in ("name", "version")},
                        "unresolved": graph["source"]["collector"].get("unresolved", 0),
                        "passed": not errors, "errors": sorted(errors)})
            row["artifact_bytes"] = {p.name: p.stat().st_size for p in dest.glob("*.json")}
            row["artifact_bytes"].update({p.name: p.stat().st_size for p in dest.glob("*.html")})
            row["graph_sha256"] = hashlib.sha256(json.dumps({k: graph[k] for k in GRAPH_FIELDS}, sort_keys=True).encode()).hexdigest()
        except (OSError, RuntimeError, ValueError, KeyError, IndexError) as error:
            row.update({"passed": False, "errors": [str(error)]})
        results.append(row)
        print(json.dumps(row), flush=True)
    report = {"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "platform": platform.platform(), "machine": platform.machine(),
              "python": platform.python_version(), "runs": args.runs,
              "rustup_toolchain": os.environ.get("RUSTUP_TOOLCHAIN"), "benchmarks": results}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    return 0 if all(r["passed"] for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
