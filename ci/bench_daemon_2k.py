#!/usr/bin/env python3
"""Measure fresh daemon responses on a reproducible, Git-backed 2,000-file fixture."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time

from bench_daemon import quantiles, rss_kib, run, wait_graph


def generate(root):
    root.mkdir(parents=True, exist_ok=False)
    (root / "package.json").write_text('{"name":"archgraph-2k-fixture","type":"module"}\n')
    for module in range(100):
        directory = root / "src" / f"m{module:03d}"
        directory.mkdir(parents=True)
        for file in range(20):
            if file < 19:
                statement = f'import "./f{file + 1:02d}";\n'
            elif module < 99:
                statement = f'import "../m{module + 1:03d}/f00";\n'
            else:
                statement = ""
            (directory / f"f{file:02d}.ts").write_text(statement + "export const value = 0;\n")
    git = ["git", "-C", str(root), "-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false",
           "-c", "commit.gpgsign=false", "-c", "user.name=Archgraph Benchmark",
           "-c", "user.email=benchmark@example.invalid"]
    run([*git, "init", "--quiet"])
    run([*git, "add", "package.json", "src"])
    subprocess.run([*git, "commit", "--quiet", "--no-gpg-sign", "-m", "baseline"], check=True,
                   env={**os.environ, "GIT_AUTHOR_DATE": "2025-01-01T00:00:00Z",
                        "GIT_COMMITTER_DATE": "2025-01-01T00:00:00Z"})
    return run([*git, "rev-parse", "HEAD"])[0].decode().strip()


def benchmark(binary, work, repeats, timeout):
    root = work / "fixture-2k"
    commit = generate(root)
    flags = ["--engine", "native", "--language", "typescript", "--depth", "2"]
    data, _ = run([binary, "--no-daemon", "scan", ".", *flags], root, timeout)
    doc = json.loads(data)
    assert (len(doc["files"]), len(doc["modules"]), len(doc["edges"])) == (2000, 100, 1999)
    cold = [run([binary, "--no-daemon", "--brief", "scan", ".", *flags], root, timeout)[1]
            for _ in range(repeats)]
    file = root / "src/m000/f00.ts"
    original = file.read_bytes()
    with (work / "daemon.log").open("wb") as log:
        process = subprocess.Popen([str(binary), "serve", ".", "--port=0", *flags], cwd=root,
                                   stdout=log, stderr=log)
        try:
            deadline = time.perf_counter() + timeout
            while not (root / ".archgraph/daemon.json").is_file():
                if process.poll() is not None or time.perf_counter() >= deadline:
                    raise RuntimeError(f"daemon startup failed; inspect {work / 'daemon.log'}")
                time.sleep(.005)
            idle = [run([binary, "--brief", "scan", ".", *flags], root, timeout)[1]
                    for _ in range(repeats)]

            def status():
                return json.loads(run([binary, "status"], root, timeout)[0])

            def wait_generation(previous):
                deadline = time.perf_counter() + timeout
                while True:
                    value = status()
                    if value["generation"] > previous:
                        return value
                    if time.perf_counter() >= deadline:
                        raise RuntimeError("body edit did not advance the graph generation")
                    time.sleep(.002)

            body = []
            for _ in range(repeats):
                before = status()
                started = time.perf_counter()
                file.write_bytes(original.replace(b"value = 0", b"value = 1"))
                fresh = wait_generation(before["generation"])
                elapsed = (time.perf_counter() - started) * 1000
                body.append({"edit_to_status_ms": elapsed, "status": fresh,
                             "rss_kib": rss_kib(process.pid)})
                file.write_bytes(original)
                wait_generation(fresh["generation"])
            imported = []
            expected = {("src/m000/f00.ts", "src/m050/f00.ts")}
            for _ in range(repeats):
                started = time.perf_counter()
                file.write_bytes(original + b'import "../m050/f00";\n')
                deadline = time.perf_counter() + timeout
                while True:
                    response, answer_ms = run([binary, "--brief", "scan", ".", *flags], root, timeout)
                    if json.loads(response)["counts"]["edges"] == 2000:
                        break
                    if time.perf_counter() >= deadline:
                        raise RuntimeError("import edit did not appear in the brief response")
                    time.sleep(.002)
                elapsed = (time.perf_counter() - started) * 1000
                wait_graph(root, expected, set(), process, timeout)
                imported.append({"edit_to_brief_scan_ms": elapsed, "answer_ms": answer_ms,
                                 "status": status(), "rss_kib": rss_kib(process.pid)})
                file.write_bytes(original)
                wait_graph(root, set(), expected, process, timeout)
        finally:
            file.write_bytes(original)
            process.kill()
            process.wait(timeout=10)
    body_times = [s["edit_to_status_ms"] for s in body]
    import_times = [s["edit_to_brief_scan_ms"] for s in imported]
    return {"schema": "archgraph/daemon-fixture-benchmark/v1", "git_backed": True, "commit": commit,
            "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "host": {"platform": platform.platform(), "machine": platform.machine(), "cpus": os.cpu_count()},
            "files": 2000, "modules": 100, "initial_edges": 1999, "repeats": repeats,
            "shape": "one directed chain through 100 folders of 20 files; the new import adds a forward edge m000/f00 to m050/f00 without a cycle",
            "body_command": "status, requiring generation advancement; body changes leave graph bytes unchanged",
            "import_command": "--brief scan, requiring the new edge count; exact pair checked after timing",
            "cold_brief_scan_ms": cold, "cold": quantiles(cold),
            "idle_brief_scan_ms": idle, "idle": quantiles(idle),
            "body_samples": body, "body": quantiles(body_times),
            "import_samples": imported, "import": quantiles(import_times)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--boundrail", "--archgraph", dest="archgraph", metavar="BINARY", type=Path, default=Path(__file__).resolve().parents[1] / "target/release/boundrail")
    parser.add_argument("--work", type=Path, help="new artifact directory; the fixture subdirectory must not exist")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=60)
    args = parser.parse_args()
    if args.repeats < 1 or args.timeout <= 0:
        parser.error("--repeats and --timeout must be positive")
    binary = args.archgraph.resolve(strict=True)
    work = args.work.resolve() if args.work else Path(tempfile.mkdtemp(prefix="archgraph-daemon-2k-"))
    work.mkdir(parents=True, exist_ok=True)
    result = benchmark(binary, work, args.repeats, args.timeout)
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    destination = args.output or work / "results.json"
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(encoded)
    print(encoded, end="")


if __name__ == "__main__":
    main()
