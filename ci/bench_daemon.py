#!/usr/bin/env python3
"""Cold versus verified edit-to-answer latency on three pinned showcase repositories."""
import argparse
import hashlib
import json
import math
import os
import platform
from pathlib import Path
import signal
import statistics
import subprocess
import tempfile
import time

REPO = Path(__file__).resolve().parents[1]
NAMES = ("next.js", "django", "typeorm")


def run(args, cwd=None, timeout=180):
    started = time.perf_counter()
    result = subprocess.run([str(a) for a in args], cwd=cwd, capture_output=True, timeout=timeout)
    elapsed = (time.perf_counter() - started) * 1000
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {args}\n{result.stderr.decode(errors='replace')}")
    return result.stdout, elapsed


def quantiles(samples):
    values = sorted(samples)
    return {"p50_ms": round(statistics.median(values), 3),
            "p95_ms": round(values[max(0, math.ceil(.95 * len(values)) - 1)], 3)}


def rss_kib(pid):
    if os.name != "posix":
        return None
    try:
        return int(subprocess.check_output(["ps", "-o", "rss=", "-p", str(pid)]).strip())
    except (OSError, ValueError, subprocess.CalledProcessError):
        return None


def prepare(entry, args, work):
    destination = work / "repos" / entry["name"]
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        raise RuntimeError(f"benchmark checkout exists: {destination}; choose a new --work directory")
    cached = args.cache / entry["name"] if args.cache else None
    source = cached if cached and cached.is_dir() else entry["repo"]
    command = ["git", "clone", "--quiet", "--no-checkout"]
    if isinstance(source, Path):
        command.append("--shared")
    run(command + [source, destination], timeout=args.timeout)
    run(["git", "checkout", "--quiet", "--detach", entry["commit"]], destination, args.timeout)
    head, _ = run(["git", "rev-parse", "HEAD"], destination)
    assert head.decode().strip() == entry["commit"]
    return destination / entry["path"]


def edit_plan(doc, language, count):
    """Add provably new, literal internal imports to selected existing source files."""
    suffixes = (".py",) if language == "python" else (".ts", ".tsx", ".js", ".jsx", ".mts", ".mjs")
    files = sorted(f["path"] for f in doc["files"] if f["path"].endswith(suffixes)
                   and not f["path"].endswith(".d.ts"))
    outgoing = {f: 0 for f in files}
    pairs = {(e["from"], e["to"]) for e in doc["edges"]}
    for source, _ in pairs:
        if source in outgoing:
            outgoing[source] += 1
    metadata = {f["path"]: f for f in doc["files"]}
    targets = sorted((f for f in files if Path(f).stem not in ("__init__", "index")
                      and not metadata.get(f, {}).get("barrel", False)), key=lambda f: (outgoing[f], f))
    for target in targets:
        sources = [f for f in files if f != target and (f, target) not in pairs
                   and Path(f).stem not in ("__init__", "index") and not metadata[f].get("barrel", False)][:count]
        if len(sources) != count:
            continue
        plan = []
        for source in sources:
            if language == "python":
                module = target.removesuffix(".py").replace("/", ".")
                if not all(part.isidentifier() for part in module.split(".")):
                    break
                addition = f"\nimport {module}  # archgraph daemon benchmark\n"
            else:
                relative = os.path.relpath(str(Path(target).with_suffix("")), str(Path(source).parent)).replace(os.sep, "/")
                if not relative.startswith("."):
                    relative = "./" + relative
                addition = f"\nimport {json.dumps(relative)}; // archgraph daemon benchmark\n"
            plan.append((source, target, addition.encode()))
        if len(plan) == count:
            return plan
    raise RuntimeError(f"could not select {count} source files with new literal imports")


def load(path):
    return json.loads(path.read_bytes())


def wait_graph(root, expected, absent, process, timeout):
    """Watcher completion is established by changed edges, never by a fast stale answer."""
    deadline = time.perf_counter() + timeout
    while time.perf_counter() < deadline:
        if process.poll() is not None:
            raise RuntimeError("daemon exited while waiting for changed imports")
        try:
            doc = load(root / ".archgraph/daemon-graph.json")
            pairs = {(e["from"], e["to"]) for e in doc["edges"]}
            if expected <= pairs and not (absent & pairs):
                return doc
        except (FileNotFoundError, json.JSONDecodeError):
            pass
        time.sleep(.002)
    raise RuntimeError(f"watcher did not publish expected imports within {timeout}s")


def bench(entry, args, work):
    root = prepare(entry, args, work)
    flags = ["--engine", args.engine, "--language", entry["language"], *entry.get("flags", [])]
    for option in ("depcruise", "python"):
        value = getattr(args, option)
        if value:
            flags += ["--" + option, str(value)]
    data, _ = run([args.archgraph, "--no-daemon", "scan", ".", *flags], root, args.timeout)
    doc = json.loads(data)
    cold = []
    for _ in range(args.repeats):
        _, elapsed = run([args.archgraph, "--no-daemon", "--brief", "scan", ".", *flags], root, args.timeout)
        cold.append(elapsed)
    baseline_pairs = {(e["from"], e["to"]) for e in doc["edges"]}
    log_path = work / (entry["name"] + "-daemon.log")
    result = {"repo": entry["name"], "commit": entry["commit"], "root": entry["path"],
              "engine": args.engine, "files": len(doc["files"]), "cold_scan_ms": cold,
              "cold": quantiles(cold), "warm": {}}
    with log_path.open("wb") as log:
        process = subprocess.Popen([str(args.archgraph), "serve", ".", "--port=0", *flags], cwd=root, stdout=log, stderr=log)
        try:
            started = time.perf_counter()
            descriptor = root / ".archgraph/daemon.json"
            while not descriptor.is_file():
                if process.poll() is not None:
                    raise RuntimeError(f"daemon warmup failed: {log_path.read_text(errors='replace')}")
                if time.perf_counter() - started > args.timeout:
                    raise RuntimeError("daemon readiness timed out")
                time.sleep(.005)
            result["daemon_start_ms"] = round((time.perf_counter() - started) * 1000, 3)
            idle = [run([args.archgraph, "--brief", "scan", ".", *flags], root, args.timeout)[1]
                    for _ in range(args.repeats)]
            result["warm_idle_ms"] = idle
            result["warm_idle"] = quantiles(idle)
            result["warm_idle_rss_kib"] = rss_kib(process.pid)
            for count in (1, 50):
                plan = edit_plan(doc, entry["language"], count)
                expected = {(source, target) for source, target, _ in plan}
                assert not (expected & baseline_pairs)
                samples = []
                originals = {source: (root / source).read_bytes() for source, _, _ in plan}
                try:
                    for _ in range(args.repeats):
                        started = time.perf_counter()
                        for source, _, addition in plan:
                            (root / source).write_bytes(originals[source] + addition)
                        deadline = time.perf_counter() + args.timeout
                        while True:
                            response, command_ms = run([args.archgraph, "--brief", "scan", ".", *flags], root, args.timeout)
                            if json.loads(response)["counts"]["edges"] == len(doc["edges"]) + len(expected):
                                elapsed = (time.perf_counter() - started) * 1000
                                break
                            if time.perf_counter() > deadline:
                                raise RuntimeError("brief scan did not answer with the changed edge count")
                            time.sleep(.002)
                        # Exact edge verification follows timing, so parsing a full snapshot cannot
                        # inflate the measured latency. The response count already proves freshness.
                        changed = wait_graph(root, expected, set(), process, args.timeout)
                        status_data, _ = run([args.archgraph, "status"], root, args.timeout)
                        status = json.loads(status_data)
                        samples.append({"edit_to_answer_ms": round(elapsed, 3), "answer_ms": round(command_ms, 3),
                                        "status": status, "daemon_rss_kib": rss_kib(process.pid),
                                        "verified_new_edges": len(expected), "files": len(changed["files"])})
                        for source, original in originals.items():
                            (root / source).write_bytes(original)
                        wait_graph(root, set(), expected, process, args.timeout)
                    result["warm"][str(count)] = {"samples": samples,
                        "edits": [{"from": source, "to": target, "line": originals[source].count(b"\n") + 2}
                                  for source, target, _ in plan],
                        **quantiles([s["edit_to_answer_ms"] for s in samples])}
                finally:
                    for source, original in originals.items():
                        (root / source).write_bytes(original)
        finally:
            process.kill()
            process.wait(timeout=10)
    (work / (entry["name"] + "-results.json")).write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--boundrail", "--archgraph", dest="archgraph", metavar="BINARY", type=Path, default=REPO / "target/release/boundrail")
    parser.add_argument("--cache", type=Path, help="existing pristine showcase repository clones; originals are never edited")
    parser.add_argument("--work", type=Path, help="new directory for isolated checkouts and daemon logs")
    parser.add_argument("--output", type=Path, help="also save the JSON measurements here")
    parser.add_argument("--engine", choices=("native", "reference", "depcruise"), default="native")
    parser.add_argument("--depcruise", type=Path)
    parser.add_argument("--python", type=Path)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--repos", nargs="+", choices=NAMES, default=NAMES)
    args = parser.parse_args()
    if args.repeats < 1 or args.timeout <= 0:
        parser.error("--repeats and --timeout must be positive")
    args.archgraph = args.archgraph.resolve(strict=True)
    if args.cache:
        args.cache = args.cache.resolve(strict=True)
    for name in ("depcruise", "python"):
        if getattr(args, name):
            setattr(args, name, getattr(args, name).resolve(strict=True))
    work = args.work.resolve() if args.work else Path(tempfile.mkdtemp(prefix="archgraph-daemon-bench-"))
    work.mkdir(parents=True, exist_ok=True)
    entries = json.loads((REPO / "showcase/repos.json").read_text())
    selected = [e for name in args.repos for e in entries if e["name"] == name]
    result = {"schema": "archgraph/daemon-benchmark/v1", "engine": args.engine,
              "mode": "native incremental" if args.engine == "native" else "reference full-refresh fallback",
              "binary": str(args.archgraph), "binary_sha256": hashlib.sha256(args.archgraph.read_bytes()).hexdigest(),
              "host": {"platform": platform.platform(), "machine": platform.machine(), "cpus": os.cpu_count()},
              "work": str(work), "repeats": args.repeats,
              "method": "Wall clock from first file write until a brief scan response has the exact newly-added edge count; exact added pairs are then verified outside timing. Cold and warm use the same brief scan command; cold bypasses daemon and retains OS filesystem caches. Restores are verified before the next sample. p95 is nearest-rank; small samples are observational.",
              "repos": [bench(entry, args, work) for entry in selected]}
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded)
    print(encoded, end="")


if __name__ == "__main__":
    main()
