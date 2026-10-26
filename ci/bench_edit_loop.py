#!/usr/bin/env python3
"""Exercise the scripted edit gate and measure fresh checks on a 2,000-file repo."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import tempfile
import time

from bench_daemon import quantiles
from bench_daemon_2k import generate


def invoke(binary, root, args, timeout, expected=0, cold=False):
    command = [str(binary), *(["--no-daemon"] if cold else []), *map(str, args)]
    env = {key: value for key, value in os.environ.items() if key != "BOUNDRAIL_BYPASS"}
    started = time.perf_counter()
    result = subprocess.run(command, cwd=root, capture_output=True, timeout=timeout, env=env)
    elapsed = (time.perf_counter() - started) * 1000
    if result.returncode != expected:
        raise RuntimeError(f"expected exit {expected}, got {result.returncode}: {command}\n"
                           f"{result.stdout.decode(errors='replace')}\n"
                           f"{result.stderr.decode(errors='replace')}")
    return result.stdout.decode(), elapsed


def capture(binary, root, rules, session, tool, file, timeout, cold=False):
    output, _ = invoke(binary, root, ["hook", "capture", "--root", root, "--agent", "claude",
        "--session", session, "--tool-id", tool, "--files", file, "--rules", rules,
        "--scope", "src/**"], timeout, cold=cold)
    value = json.loads(output)
    assert re.fullmatch("[0-9a-f]{64}", value["snapshot"]), value
    assert isinstance(value["context"], str), value
    return value


def check(binary, root, baseline, timeout, verdict, cold=False):
    output, elapsed = invoke(binary, root, ["hook", "check", "--root", root,
        "--baseline", baseline, "--brief"], timeout, expected=int(verdict == "BLOCK"), cold=cold)
    first, body = output.split("\n", 1)
    value = json.loads(body)
    assert first == value["verdict"] == verdict, output
    assert bool(value["violations"]) == (verdict == "BLOCK"), value
    for finding in value["violations"]:
        assert finding["finding_id"] and finding["recipe"], finding
    return value, elapsed


def write_rules(root, source, target):
    rules = root / "layout.yaml"
    rules.write_text('version: 1\nlayers: ["src/**"]\nforbid:\n'
        f'  - {{ id: domain-no-adapters, from: "{source}", to: "{target}", '
        'reason: "Domain logic must not depend on adapter implementations." }\n')
    return rules


def fixture(binary, work, timeout):
    root = work / "fixture-loop"
    (root / "src/domain").mkdir(parents=True)
    (root / "src/adapters").mkdir()
    (root / "package.json").write_text('{"name":"boundrail-edit-loop","type":"module"}\n')
    source = root / "src/domain/a.ts"
    original = 'export const value = 0;\n'
    violation = original + 'import "../adapters/b";\n'
    source.write_text(original)
    (root / "src/adapters/b.ts").write_text('export const adapter = 1;\n')
    rules = write_rules(root, "src/domain/**", "src/adapters/**")
    before = capture(binary, root, rules, "acceptance", "edit-1", "src/domain/a.ts", timeout)
    source.write_text(violation)
    blocked, _ = check(binary, root, before["snapshot"], timeout, "BLOCK")
    repair = capture(binary, root, rules, "acceptance", "repair-1", "src/domain/a.ts", timeout)
    assert repair["snapshot"] == before["snapshot"], "repair must retain the original baseline"
    still_blocked, _ = check(binary, root, repair["snapshot"], timeout, "BLOCK")
    assert [v["finding_id"] for v in blocked["violations"]] == [
        v["finding_id"] for v in still_blocked["violations"]]
    source.write_text(original)
    passed, _ = check(binary, root, before["snapshot"], timeout, "PASS")
    source.write_text(violation)
    existing = capture(binary, root, rules, "preexisting", "edit-1", "src/domain/a.ts", timeout)
    preexisting, _ = check(binary, root, existing["snapshot"], timeout, "PASS")
    output, _ = invoke(binary, root, ["fires", "--top", "--root", root], timeout)
    fires = json.loads(output)
    assert fires["total"] >= 2 and fires["rules"], fires
    transcript = {"kind": "scripted CLI session; no live agent host", "capture": before,
                  "blocked": blocked, "unchanged_repair_blocked": still_blocked,
                  "fixed": passed, "preexisting_unchanged": preexisting, "fires": fires}
    (work / "transcript.json").write_text(json.dumps(transcript, indent=2) + "\n")
    return transcript


def summary(samples):
    return {**quantiles(samples), "max_ms": round(max(samples), 3)}


def benchmark(binary, work, repeats, timeout):
    root = work / "fixture-2k"
    commit = generate(root)
    source = root / "src/m000/f00.ts"
    original = source.read_bytes()
    rules = write_rules(root, "src/m000/f00.ts", "src/m050/f00.ts")
    addition = b'import "../m050/f00";\n'
    output, _ = invoke(binary, root, ["scan", ".", "--engine", "native", "--symbols",
                                    "--rules", rules], timeout, cold=True)
    doc = json.loads(output)
    assert (len(doc["files"]), len(doc["modules"]), len(doc["edges"])) == (2000, 100, 1999)

    warm_status = []

    def samples(mode, cold, daemon=None, mcp=False):
        values = []
        for index in range(repeats):
            if daemon is not None and daemon.poll() is not None:
                raise RuntimeError("daemon exited before the warm sample")
            before = capture(binary, root, rules, f"{mode}-{index}", "edit", "src/m000/f00.ts", timeout, cold)
            prior_status = None
            if daemon is not None:
                output, _ = invoke(binary, root, ["status"], timeout)
                prior_status = json.loads(output)
            source.write_bytes(original + addition)
            if mcp:
                output, elapsed = invoke(binary, root, ["mcp-call", "--tool", "check_diff",
                    "--arguments", json.dumps({"snapshot": before["snapshot"]})], timeout, cold=cold)
                result = json.loads(output)
                assert result["verdict"] == "BLOCK" and result["violations"], result
            else:
                result, elapsed = check(binary, root, before["snapshot"], timeout, "BLOCK", cold)
            assert any(v["rule"] == "domain-no-adapters" for v in result["violations"]), result
            values.append(elapsed)
            if daemon is not None:
                output, _ = invoke(binary, root, ["status"], timeout)
                status = json.loads(output)
                assert status["running"] and status["generation"] > prior_status["generation"], status
                warm_status.append({"before_edit": prior_status, "after_check": status})
            source.write_bytes(original)
            check(binary, root, before["snapshot"], timeout, "PASS", cold)
        return values

    cold = samples("cold", True)
    mcp_cold = samples("mcp-cold", True, mcp=True)
    log_path = work / "daemon.log"
    with log_path.open("wb") as log:
        process = subprocess.Popen([str(binary), "serve", ".", "--port=0", "--engine", "native",
                                    "--symbols", "--rules", str(rules)], cwd=root, stdout=log, stderr=log,
                                   env={key: value for key, value in os.environ.items() if key != "BOUNDRAIL_BYPASS"})
        try:
            deadline = time.perf_counter() + timeout
            while not (root / ".boundrail/daemon.json").is_file():
                if process.poll() is not None or time.perf_counter() >= deadline:
                    raise RuntimeError(f"daemon startup failed; inspect {log_path}")
                time.sleep(.005)
            warm = samples("warm", False, process)
            mcp_warm = samples("mcp-warm", False, process, mcp=True)
        finally:
            source.write_bytes(original)
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
    return {"files": 2000, "modules": 100, "initial_edges": 1999, "fixture_commit": commit,
            "repeats": repeats, "cold_samples_ms": cold, "cold": summary(cold),
            "warm_samples_ms": warm, "warm": summary(warm),
            "mcp_check_diff": {"cold_samples_ms": mcp_cold, "cold": summary(mcp_cold),
                               "warm_samples_ms": mcp_warm, "warm": summary(mcp_warm),
                               "transport": "mcp-call CLI bridge; stdio envelope/host scheduling excluded"},
            "warm_daemon_status": warm_status,
            "timing": "whole hook check subprocess after a new forbidden import: freshness, graph diff, recipe, audit and response; capture and repair excluded",
            "thresholds_ms": {"warm": 300, "cold": 3000},
            "all_samples_meet_targets": max(warm + mcp_warm) < 300 and max(cold + mcp_cold) < 3000}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--boundrail", dest="boundrail", metavar="BINARY", type=Path, default=Path(__file__).resolve().parents[1] / "target/release/boundrail")
    parser.add_argument("--work", type=Path, help="new artifact directory; fixtures must not exist")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--fixture-only", action="store_true")
    args = parser.parse_args()
    if args.repeats < 1 or args.timeout <= 0:
        parser.error("--repeats and --timeout must be positive")
    binary = args.boundrail.resolve(strict=True)
    work = args.work.resolve() if args.work else Path(tempfile.mkdtemp(prefix="boundrail-edit-loop-"))
    work.mkdir(parents=True, exist_ok=True)
    result = {"schema": "boundrail/edit-loop-benchmark/v1", "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "host": {"platform": platform.platform(), "machine": platform.machine(), "cpus": os.cpu_count()},
              "fixture": fixture(binary, work, args.timeout)}
    if not args.fixture_only:
        result["latency"] = benchmark(binary, work, args.repeats, args.timeout)
    destination = args.output or work / "results.json"
    destination.parent.mkdir(parents=True, exist_ok=True)
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    destination.write_text(encoded)
    print(encoded, end="")


if __name__ == "__main__":
    main()
