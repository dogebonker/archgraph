#!/usr/bin/env python3
"""Measure independently labelled service flows. This is not whole-repository accuracy.

Inputs are local pinned sources. The runner neither fetches nor executes their code.
Reports and annotations are checked before counting, and unknown predictions stay unreviewed.
"""
import argparse
import hashlib
import json
import shutil
from pathlib import Path, PurePosixPath
import subprocess
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def contained(root, relative):
    path = PurePosixPath(relative)
    if not relative or path.is_absolute() or any(p in ("", ".", "..") for p in relative.split("/")) or "\\" in relative:
        raise ValueError(f"invalid repository-relative evidence path: {relative!r}")
    resolved = (root / relative).resolve()
    if not resolved.is_relative_to(root.resolve()):
        raise ValueError(f"evidence escapes repository: {relative}")
    return resolved


def location(value):
    source = value["source"]
    if not isinstance(source["line"], int) or isinstance(source["line"], bool) or source["line"] < 1:
        raise ValueError("source line must be a positive integer")
    return source["file"], source["line"]


def edge_key(edge):
    return (edge["from"], edge["to"], edge["kind"], edge.get("operation"), *location(edge))


def validate_truth(truth, repos):
    if truth.get("version") != 1 or not truth.get("flows"):
        raise ValueError("truth must contain version 1 and a nonempty independently labelled flow set")
    names = {repo["name"] for repo in truth["repositories"]}
    if len(names) != len(truth["repositories"]):
        raise ValueError("duplicate repository name")
    if any(not name or name in {".", ".."} or "/" in name or "\\" in name or any(ord(c) < 32 for c in name) for name in names):
        raise ValueError("repository names must be single artifact directory names")
    ids, expected = set(), set()
    statuses = {}
    for repo in truth["repositories"]:
        root = contained(repos, repo.get("path", repo["name"]))
        if not root.is_dir() or not repo.get("files"):
            raise ValueError(f"missing source repository or annotated file hashes: {root}")
        scan_root = contained(root, repo["source_subdir"]) if repo.get("source_subdir") else root
        if not scan_root.is_dir():
            raise ValueError(f"missing labelled source subtree: {scan_root}")
        if "boundary_manifest" in repo:
            manifest = repo["boundary_manifest"]
            if set(manifest) != {"version", "services", "edges"} or manifest["version"] != 1 or manifest["edges"] != [] or not manifest["services"] or len(set(manifest["services"])) != len(manifest["services"]):
                raise ValueError("benchmark boundary manifests must declare services only, with edges: []")
            for service in manifest["services"]:
                if not contained(scan_root, service).is_dir():
                    raise ValueError(f"declared benchmark service directory is missing: {service}")
        pin = repo["commit"]
        if len(pin) != 40 or any(c not in "0123456789abcdef" for c in pin):
            raise ValueError(f"invalid immutable commit for {repo['name']}")
        provenance = "annotated_files_only"
        if (root / ".git").exists():
            actual = subprocess.run(["git", "--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null", "-C", str(root), "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
            if actual != pin:
                raise ValueError(f"repository {repo['name']} must be at {pin}")
            provenance = "git_head_and_annotated_files"
        for file, expected_hash in repo["files"].items():
            path = contained(scan_root, file)
            if not path.is_file() or digest(path) != expected_hash:
                raise ValueError(f"missing or stale source evidence: {repo['name']}/{file}")
        statuses[repo["name"]] = {"root": str(scan_root), "repository_root": str(root), "commit": pin, "kind": repo.get("kind", "unspecified"), "provenance": provenance, "verified_files": len(repo["files"])}
    for case in truth["flows"] + truth.get("controls", []):
        if case["id"] in ids or case["repository"] not in names:
            raise ValueError(f"duplicate case ID or unknown repository: {case['id']}")
        ids.add(case["id"])
        if not case.get("reason") or not case.get("evidence"):
            raise ValueError(f"case {case['id']} requires an explanation and source witnesses")
        repo = next(r for r in truth["repositories"] if r["name"] == case["repository"])
        root = Path(statuses[repo["name"]]["root"])
        witnesses = {(e["file"], e["line"]) for e in case["evidence"]}
        if location(case) not in witnesses:
            raise ValueError(f"caller location lacks a source witness: {case['id']}")
        if "expected" in case:
            key = (case["repository"], edge_key({**case["expected"], "source": case["source"]}))
            if key in expected or not case.get("target") or (case["target"]["file"], case["target"]["line"]) not in witnesses:
                raise ValueError(f"duplicate flow or missing receiver evidence: {case['id']}")
            expected.add(key)
        else:
            if not case.get("forbid"):
                raise ValueError(f"negative control requires an explicit forbidden prediction: {case['id']}")
        for witness in case["evidence"]:
            if witness["file"] not in repo["files"] or not witness.get("text"):
                raise ValueError(f"unhashed or empty source witness: {case['id']}")
            lines = contained(root, witness["file"]).read_text().splitlines()
            line = witness["line"]
            if not isinstance(line, int) or isinstance(line, bool) or line < 1 or line > len(lines) or lines[line - 1].strip() != witness["text"]:
                raise ValueError(f"source witness moved or changed: {case['id']} at {witness['file']}:{line}")
    return statuses


def compare(truth, reports):
    """Count only source-labelled predictions. An unlabelled edge is not a true positive."""
    required = {r["name"] for r in truth["repositories"]}
    if set(reports) != required:
        raise ValueError("every labelled repository requires one complete inference report")
    positives = {}
    controls = {}
    for case in truth["flows"]:
        positives.setdefault((case["repository"], *location(case)), []).append(case)
    for case in truth.get("controls", []):
        controls.setdefault((case["repository"], *location(case)), []).append(case)
    matched, false_positives, unreviewed, duplicate_count = set(), [], [], 0
    violated = set()
    emitted = 0
    for name, report in reports.items():
        if (report.get("schema") != "archgraph/v1.2" or report.get("kind") != "services"
                or not isinstance(report.get("edges"), list) or not isinstance(report.get("services"), list)
                or report.get("source", {}).get("collector") not in {"inferred", "inferred+declarative"}
                or report.get("source", {}).get("complete") is not False or report.get("truncated")):
            raise ValueError(f"{name} is not a complete live inferred services report")
        if report.get("summary", {}).get("calls") != len(report["edges"]):
            raise ValueError(f"{name} report call count is inconsistent")
        seen = set()
        for edge in report["edges"]:
            if edge["from"] not in report["services"] or edge["to"] not in report["services"]:
                raise ValueError(f"{name} report edge references an unknown service")
            key = edge_key(edge)
            if key in seen:
                duplicate_count += 1
                continue
            seen.add(key)
            emitted += 1
            site = (name, *location(edge))
            cases = positives.get(site, [])
            hits = [case for case in cases if key == edge_key({**case["expected"], "source": case["source"]})]
            if hits:
                matched.update(case["id"] for case in hits)
                continue
            failures = [case for case in controls.get(site, []) if all(edge.get(k) == v for k, v in case["forbid"].items())]
            prediction = {"repository": name, "edge": edge}
            if cases or failures:
                false_positives.append(prediction)
                violated.update(case["id"] for case in failures)
            else:
                unreviewed.append(prediction)
    misses = [{"id": case["id"], "repository": case["repository"], "expected": case["expected"], "source": case["source"], "configuration": case.get("configuration"), "reason": case["reason"]}
              for case in truth["flows"] if case["id"] not in matched]
    tp, fp = len(matched), len(false_positives)
    by_repository = {}
    for name in sorted(required):
        cases = [case for case in truth["flows"] if case["repository"] == name]
        found = sum(case["id"] in matched for case in cases)
        wrong = sum(edge["repository"] == name for edge in false_positives)
        repo = next(r for r in truth["repositories"] if r["name"] == name)
        by_repository[name] = {"kind": repo.get("kind", "unspecified"), "expected_flows": len(cases), "matched_flows": found,
                               "precision": found / (found + wrong) if found + wrong else None,
                               "recall": found / len(cases) if cases else None,
                               "false_positives": wrong}
    return {"scope": "independently labelled source callsites under stated configuration assumptions",
            "expected_flows": len(truth["flows"]), "matched_flows": tp, "matched_ids": sorted(matched),
            "by_repository": by_repository,
            "labelled_predictions": tp + fp, "false_positives": false_positives,
            "precision": tp / (tp + fp) if tp + fp else None,
            "recall": tp / len(truth["flows"]), "misses": misses,
            "controls": {"total": len(truth.get("controls", [])), "violated": sorted(violated)},
            "unreviewed_predictions": unreviewed, "unique_predictions": emitted,
            "duplicate_predictions": duplicate_count,
            "needs_prediction_review": bool(unreviewed),
            "all_labelled_flows_recovered": not misses}


def scratch_path(path, evidence):
    path = path.resolve()
    if path.is_relative_to(ROOT) or any(path.is_relative_to(Path(v["repository_root"])) for v in evidence.values()):
        raise ValueError("benchmark artifacts must be outside the checkout and source repositories")
    return path


def run(args):
    truth_bytes = args.truth.read_bytes()
    truth = json.loads(truth_bytes)
    truth_hash = hashlib.sha256(truth_bytes).hexdigest()
    evidence = validate_truth(truth, args.repos)
    reports, runs = {}, {}
    binary = args.archgraph.resolve(strict=True)
    output = scratch_path(args.output, evidence)
    artifacts = scratch_path(output.parent / (output.stem + "-reports"), evidence)
    artifacts.mkdir(parents=True, exist_ok=True)
    frozen_binary = scratch_path(artifacts / "archgraph", evidence)
    shutil.copy2(binary, frozen_binary)
    binary_hash = digest(frozen_binary)
    for name, source in evidence.items():
        run_dir = scratch_path(artifacts / name, evidence)
        run_dir.mkdir(exist_ok=True)
        command = [str(frozen_binary), "services", source["root"], "--format", "json"]
        repo = next(r for r in truth["repositories"] if r["name"] == name)
        if "boundary_manifest" in repo:
            manifest_path = scratch_path(run_dir / "boundaries.json", evidence)
            manifest_path.write_text(json.dumps(repo["boundary_manifest"], indent=2) + "\n")
            command.extend(["--manifest", str(manifest_path)])
        start = time.monotonic()
        process = subprocess.run(command, capture_output=True, text=True, timeout=args.timeout)
        elapsed = time.monotonic() - start
        scratch_path(run_dir / "stderr.txt", evidence).write_text(process.stderr)
        if process.returncode:
            raise RuntimeError(f"{name} inference exited {process.returncode}: {process.stderr[-2000:]}")
        report = json.loads(process.stdout)
        scratch_path(run_dir / "report.json", evidence).write_text(process.stdout)
        reports[name] = report
        runs[name] = {"command": command, "exit_code": process.returncode, "observed_seconds": round(elapsed, 3)}
    result = {"truth_sha256": truth_hash, "binary_sha256": binary_hash, "source_evidence": evidence,
              "runs": runs, "comparison": compare(truth, reports)}
    output.write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archgraph", type=Path, required=True)
    parser.add_argument("--repos", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--truth", type=Path, default=HERE / "service-flows.json")
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    try:
        result = run(args)
    except (ValueError, KeyError, OSError, RuntimeError, subprocess.SubprocessError) as error:
        parser.exit(1, f"service flow benchmark failed: {error}\n")
    print(json.dumps(result["comparison"], indent=2))


if __name__ == "__main__":
    main()
