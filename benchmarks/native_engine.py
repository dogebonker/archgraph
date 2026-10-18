#!/usr/bin/env python3
"""Compare native/reference collectors on the first 20 pinned showcase repositories.

Artifacts live outside the checkout. No package installation or target code execution.
Agreement measures reference compatibility, not ground-truth correctness.
"""
import argparse
import concurrent.futures
import importlib.util
import json
from pathlib import Path
import statistics
import subprocess
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
_spec = importlib.util.spec_from_file_location("showcase_edge_quality", ROOT / "showcase/edge_quality.py")
_showcase = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_showcase)
prepare = _showcase.prepare
digest = _showcase.digest
write_json = _showcase.write_json


def kind(value):
    return {"static": "runtime", "dynamic": "lazy", "type-reexport": "type"}.get(value, value or "runtime")


def edge_index(document, detailed=False, cohort=None):
    files = {file["path"] for file in document["files"]}
    result = {}
    for edge in document["edges"]:
        if edge["from"] not in files or edge["to"] not in files:
            continue
        if cohort is not None and not {edge["from"], edge["to"]} <= cohort:
            continue
        key = (edge["from"], edge["to"])
        if detailed:
            key += (kind(edge.get("kind")), edge.get("original_to"))
        result.setdefault(key, []).append(edge)
    return result


def agreement(reference, native):
    common = len(reference & native)
    # Empty/empty agrees; an empty prediction cannot satisfy recall on a nonempty reference.
    return {"reference": len(reference), "native": len(native), "common": common,
            "precision": common / len(native) if native else (1.0 if not reference else 0.0),
            "recall": common / len(reference) if reference else 1.0,
            "jaccard": common / len(reference | native) if reference | native else 1.0}


def compare(reference, native):
    ref_files = {f["path"] for f in reference["files"]}
    new_files = {f["path"] for f in native["files"]}
    result = {"missing_files": sorted(ref_files - new_files), "extra_files": sorted(new_files - ref_files),
              "metrics": {}, "disagreements": {}}
    for label, cohort in (("all_files", None), ("shared_files", ref_files & new_files)):
        result["metrics"][label] = {}
        for mode, detailed in (("endpoints", False), ("kind_and_original_target", True)):
            old, new = edge_index(reference, detailed, cohort), edge_index(native, detailed, cohort)
            result["metrics"][label][mode] = agreement(set(old), set(new))
            if cohort is None:
                # Preserve every witness, including duplicate source occurrences, without guessing causes.
                result["disagreements"][mode] = {
                    "reference_only": [{"key": list(key), "reason": "unclassified", "evidence": old[key]}
                                       for key in sorted(old.keys() - new.keys(), key=repr)],
                    "native_only": [{"key": list(key), "reason": "unclassified", "evidence": new[key]}
                                    for key in sorted(new.keys() - old.keys(), key=repr)]}
    for engine, document in (("reference", reference), ("native", native)):
        cycles = [c for c in document.get("cycles", []) if c["level"] == "module"]
        result[engine] = {"files": len(document["files"]), "edges": len(edge_index(document)),
                          "resolution": document.get("import_resolution"),
                          "unresolved": document.get("unresolved"),
                          "module_cycles": len(cycles),
                          "largest_module_cycle": max((len(c["members"]) for c in cycles), default=0)}
    metrics = result["metrics"]["all_files"]["endpoints"]
    result["numeric_gate_passes"] = metrics["precision"] > .98 and metrics["recall"] > .98
    result["default_flip_ready"] = False  # Scope, resolution and disagreement review remain human decisions.
    return result


def scan(entry, args, binary_hash):
    destination = args.work / "results" / entry["name"]
    destination.mkdir(parents=True, exist_ok=True)
    saved = {"entry": entry, "status": "error", "runs": {}}
    try:
        root = prepare(entry, args.work, args.cache)
        documents = {}
        # Same binary, root and flags; alternating order reduces systematic warm-cache preference.
        engines = ("reference", "native") if args.order == "reference-first" else ("native", "reference")
        for repeat in range(args.repeat):
            for engine in engines if repeat % 2 == 0 else tuple(reversed(engines)):
                run_dir = destination / engine / str(repeat + 1)
                run_dir.mkdir(parents=True, exist_ok=True)
                graph = run_dir / "graph.json"
                collector = (["--depcruise", str(args.depcruise)] if entry["language"] == "typescript"
                             else ["--python", str(args.python)]) if engine == "reference" else []
                command = [str(args.archgraph), "scan", str(root / entry["path"]), "--language", entry["language"],
                           "--engine", engine, *collector, *entry["flags"], "-o", str(graph)]
                run = {"command": command}
                saved["runs"].setdefault(engine, []).append(run)
                print(f"{entry['name']}: {engine} scan {repeat + 1}/{args.repeat}", flush=True)
                started = time.monotonic()
                with (run_dir / "scan.log").open("wb") as log:
                    process = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=args.timeout)
                run.update(seconds=time.monotonic() - started, exit_code=process.returncode)
                if process.returncode:
                    raise RuntimeError(f"{engine} scan exited {process.returncode}: {run_dir / 'scan.log'}")
                document = json.loads(graph.read_text())
                if document.get("source", {}).get("git", {}).get("commit") != entry["commit"]:
                    raise RuntimeError("scan commit differs from manifest pin")
                if document["source"]["git"].get("dirty"):
                    raise RuntimeError("scan reports modified tracked source")
                run.update(graph_sha256=digest(graph), collector=document["source"]["collector"])
                documents[engine] = document
        if digest(args.archgraph) != binary_hash:
            raise RuntimeError("binary changed during comparison; rerun with a frozen binary")
        saved.update(status="ok", comparison=compare(documents["reference"], documents["native"]))
        saved["median_seconds"] = {engine: statistics.median(r["seconds"] for r in runs)
                                    for engine, runs in saved["runs"].items()}
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        saved["error"] = str(error)
    write_json(destination / "comparison.json", saved)
    print(f"{entry['name']}: {saved['status']}" + (f" — {saved['error']}" if "error" in saved else ""), flush=True)
    return saved


def markdown(result):
    successful = [row for row in result["repositories"] if row["status"] == "ok"]
    passed = sum(row["comparison"]["numeric_gate_passes"] for row in successful)
    reference_seconds = sum(row["median_seconds"]["reference"] for row in successful)
    native_seconds = sum(row["median_seconds"]["native"] for row in successful)
    lines = ["# Native engine comparison", "", "Generated from actual scans at the showcase manifest pins. "
             "Reference compatibility is not a correctness score. The default remains the reference engine.", "",
             f"{len(successful)}/{len(result['repositories'])} repositories scanned successfully; "
             f"{passed}/{len(result['repositories'])} exceed 98% endpoint precision and recall. "
             f"Sum of per-repository median scan times: reference {reference_seconds:.3f}s, native {native_seconds:.3f}s.", "",
             f"Binary SHA-256: `{result['binary_sha256']}`. Cargo.lock SHA-256: `{result['lock_sha256']}`.", "",
             f"Timing: {result['repeat']} end-to-end scan(s) per engine, median seconds; parsing, resolution and analysis included. "
             "No isolated parser speed claim. Local caches, run order and concurrent validation on the development host affect wall time; "
             "these timings are observational.", "",
             "| Repository | Files ref/native | Internal edges ref/native | Precision / recall | Shared-file P/R | Unresolved ref/native | Cycles ref/native | Largest ref/native | Seconds ref/native | >98% both |",
             "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |"]
    for row in result["repositories"]:
        if row["status"] != "ok":
            lines.append(f"| {row['entry']['name']} | FAILED: {row.get('error', 'unknown').replace('|', '/')} |")
            continue
        c = row["comparison"]
        r, n = c["reference"], c["native"]
        m, s = c["metrics"]["all_files"]["endpoints"], c["metrics"]["shared_files"]["endpoints"]
        unresolved = lambda stats: str(stats["resolution"]["unresolved"]) if stats["resolution"] else "unknown"
        seconds = row["median_seconds"]
        lines.append(f"| {row['entry']['name']} | {r['files']}/{n['files']} | {r['edges']}/{n['edges']} | "
                     f"{m['precision']:.2%} / {m['recall']:.2%} | {s['precision']:.2%} / {s['recall']:.2%} | "
                     f"{unresolved(r)}/{unresolved(n)} | {r['module_cycles']}/{n['module_cycles']} | "
                     f"{r['largest_module_cycle']}/{n['largest_module_cycle']} | {seconds['reference']:.3f}/{seconds['native']:.3f} | "
                     f"{'yes' if c['numeric_gate_passes'] else 'no'} |")
    lines += ["", "## Scope and disagreements", "",
              "All-file metrics include missing files, so dropping files cannot inflate recall. Shared-file metrics are diagnostic only. "
              "Endpoints are deduplicated directed internal file edges; kind/`original_to` agreement is recorded separately in JSON. "
              "Known external packages are excluded from the internal-edge score, but unresolved occurrences are retained.", "",
              "Every differing edge and its available source line/specifier is retained as unclassified evidence in each "
              "`results/<repo>/comparison.json`, beside raw graphs and scan logs. Causes require source inspection; "
              "a missing edge is not automatically a native bug. Scope changes, unknown resolution and unreviewed differences "
              "block a default flip even when the numeric gate passes. "
              "[Source review notes](native-engine-review.md) explain representative cases and remaining limits.", ""]
    for row in result["repositories"]:
        if row["status"] == "ok":
            c = row["comparison"]
            d = c["disagreements"]["endpoints"]
            typed = c["metrics"]["all_files"]["kind_and_original_target"]
            lines.append(f"- **{row['entry']['name']}**: {len(c['missing_files'])} missing / {len(c['extra_files'])} extra files; "
                         f"{len(d['reference_only'])} reference-only / {len(d['native_only'])} native-only edges, unclassified. "
                         f"Kind/original-target precision {typed['precision']:.2%}, recall {typed['recall']:.2%}.")
    lines += ["", "## Reproduce", "", "```sh", result["reproduce"], "```", "",
              f"Full machine-readable manifest, commands, collector metadata and hashes: `{result['artifact_root']}/comparison.json`.", ""]
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archgraph", type=Path, required=True)
    parser.add_argument("--work", type=Path, default=Path("/private/tmp/archgraph-native-comparison"))
    parser.add_argument("--cache", type=Path, default=Path("/private/tmp/archgraph-edge-quality/repos"))
    parser.add_argument("--depcruise", type=Path, default=Path("/private/tmp/archgraph-impact-collectors/node_modules/.bin/depcruise"))
    parser.add_argument("--python", type=Path, default=Path("/private/tmp/archgraph-impact-python/bin/python"))
    parser.add_argument("--only", action="append", default=[])
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--order", choices=("reference-first", "native-first"), default="reference-first")
    parser.add_argument("--report", type=Path, default=ROOT / "docs/native-engine-comparison.md")
    args = parser.parse_args(argv)
    if min(args.jobs, args.repeat, args.timeout) < 1:
        parser.error("jobs, repeat and timeout must be positive")
    args.archgraph, args.work, args.cache = args.archgraph.resolve(), args.work.resolve(), args.cache.resolve()
    manifest = ROOT / "showcase/repos.json"
    entries = json.loads(manifest.read_text())[:20]
    unknown = set(args.only) - {e["name"] for e in entries}
    if unknown:
        parser.error(f"unknown first-20 repository names: {sorted(unknown)}")
    entries = [e for e in entries if not args.only or e["name"] in args.only]
    binary_hash = digest(args.archgraph)
    result = {"binary_sha256": binary_hash, "lock_sha256": digest(ROOT / "Cargo.lock"),
              "manifest_sha256": digest(manifest), "repeat": args.repeat, "order": args.order,
              "artifact_root": str(args.work), "repositories": []}
    import shlex
    command = ["python3", "-B", "benchmarks/native_engine.py", "--archgraph", str(args.archgraph),
               "--work", str(args.work), "--cache", str(args.cache), "--depcruise", str(args.depcruise),
               "--python", str(args.python), "--jobs", str(args.jobs), "--repeat", str(args.repeat), "--order", args.order]
    for name in args.only:
        command += ["--only", name]
    result["reproduce"] = shlex.join(command)
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(scan, entry, args, binary_hash) for entry in entries]
        result["repositories"] = [future.result() for future in futures]
    write_json(args.work / "comparison.json", result)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(markdown(result))
    return int(any(row["status"] != "ok" for row in result["repositories"]))


if __name__ == "__main__":
    raise SystemExit(main())
