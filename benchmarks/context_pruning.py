#!/usr/bin/env python3
"""Reproducible base-commit, held-out-file context retrieval experiment (stdlib only)."""

import argparse
import hashlib
import json
import math
import os
import re
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path, PurePosixPath

REPO = Path(__file__).resolve().parents[1]
WORK = Path("/private/tmp/boundrail-context-validation")
PROTOCOL = {
    "version": 1,
    "population": "First 20 showcase repositories; latest merged PRs in first 500 closed pulls returned at collection time, then descending merged_at and PR number.",
    "amendment": "Before any retrieval scoring, the 100-closed-PR pilot yielded 29/30 cases (no SQLAlchemy eligible PR); expanded uniformly to 500 closed PRs for all 20 repositories.",
    "allocation": "One eligible PR per repository plus a second for the first ten manifest repositories; no replacements after scoring.",
    "eligibility": "At least two modified/renamed existing production source files in the scanned base graph, excluding pure barrels and index.ts. Added/deleted/out-of-root/non-source/default-excluded files cannot be held out or seeded; all changes and exclusions retained. The recorded base must be an ancestor of the PR merge commit; other API bases are rejected to prevent future-graph leakage.",
    "heldout": 'Sorted eligible paths indexed by int(sha256(repository + "/" + PR number),16) modulo count.',
    "task": "PR title only; no body, head graph, diff contents or held-out path supplied to either ranker.",
    "budget": 40000,
    "baseline": "Seeds first, then case-insensitive title keyword matches in paths or file contents, then files in seed folders; keyword hit count descending, folder distance ascending, path ascending. Greedy ceil((UTF8 path bytes + UTF8 reason bytes + body bytes +160)/4), same task overhead and constraints first. No graph expansion.",
    "metric": "One held-out observation per PR: path appears in emitted entries within budget; signatures count as retrieval, not full-body reading.",
    "frozen_before_scoring": True,
}


def run(command, timeout=300):
    return subprocess.run(
        [str(x) for x in command],
        check=True,
        text=True,
        capture_output=True,
        timeout=timeout,
    ).stdout


def dump(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def api(endpoint, cache):
    if not cache.exists():
        cache.parent.mkdir(parents=True, exist_ok=True)
        cache.write_text(run(["gh", "api", endpoint], 90))
    return json.loads(cache.read_text())


def relative_source(path, entry):
    root = entry["path"].rstrip("/")
    if root != ".":
        if not path.startswith(root + "/"):
            return None
        path = path[len(root) + 1 :]
    suffixes = (
        (".py",)
        if entry["language"] == "python"
        else (".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts")
    )
    return path if path.endswith(suffixes) else None


def heldout(repo, number, paths):
    paths = sorted(paths)
    return paths[
        int(hashlib.sha256(f"{repo}/{number}".encode()).hexdigest(), 16) % len(paths)
    ]


def prepare(entry):
    clone = WORK / "repos" / entry["name"]
    if not clone.exists():
        clone.parent.mkdir(parents=True, exist_ok=True)
        cache = Path("/private/tmp/boundrail-edge-quality/repos") / entry["name"]
        if cache.exists():
            run(["git", "clone", "--shared", "--no-checkout", cache, clone], 120)
        else:
            run(
                [
                    "git",
                    "clone",
                    "--filter=blob:none",
                    "--no-checkout",
                    entry["repo"],
                    clone,
                ],
                300,
            )
        run(["git", "-C", clone, "remote", "set-url", "origin", entry["repo"]])
    if not clone.resolve().is_relative_to(WORK.resolve()):
        raise ValueError("experiment clone must stay inside its work directory")
    common = Path(
        run(
            [
                "git",
                "-C",
                clone,
                "rev-parse",
                "--path-format=absolute",
                "--git-common-dir",
            ]
        ).strip()
    )
    if not common.resolve().is_relative_to(WORK.resolve()):
        raise ValueError("experiment Git directory must not belong to a shared cache")
    run(["git", "-C", clone, "config", "remote.origin.promisor", "true"])
    run(["git", "-C", clone, "config", "remote.origin.partialclonefilter", "blob:none"])
    return clone


def scan(entry, pull, binary, options):
    case = WORK / "cases" / f"{entry['name']}-{pull['number']}"
    case.mkdir(parents=True, exist_ok=True)
    base = pull["base"]["sha"]
    clone = prepare(entry)
    merge = pull["merge_commit_sha"]
    for commit in [base, merge]:
        try:
            run(["git", "-C", clone, "cat-file", "-e", commit + "^{commit}"])
        except subprocess.CalledProcessError:
            run(
                [
                    "git",
                    "-C",
                    clone,
                    "-c",
                    "fetch.negotiationAlgorithm=noop",
                    "fetch",
                    "--no-tags",
                    "origin",
                    commit,
                ],
                180,
            )
    run(["git", "-C", clone, "merge-base", "--is-ancestor", base, merge], 90)
    checkout = case / "checkout"
    if not (case / "scan-symbols.json").exists():
        clone = prepare(entry)
        try:
            run(["git", "-C", clone, "cat-file", "-e", base + "^{commit}"])
        except subprocess.CalledProcessError:
            run(["git", "-C", clone, "fetch", "--no-tags", "origin", base], 180)
        # Batch missing blobs before checkout; single-object lazy fetches are prohibitively slow.
        objects = run(
            [
                "git",
                "-C",
                clone,
                "rev-list",
                "--objects",
                "--no-walk",
                "--missing=print",
                base,
            ],
            180,
        )
        missing = [line[1:] for line in objects.splitlines() if line.startswith("?")]
        for offset in range(0, len(missing), 4000):
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(clone),
                    "-c",
                    "fetch.negotiationAlgorithm=noop",
                    "fetch",
                    "--quiet",
                    "--no-tags",
                    "--no-write-fetch-head",
                    "--no-auto-maintenance",
                    "--recurse-submodules=no",
                    "--stdin",
                    "origin",
                ],
                input="\n".join(missing[offset : offset + 4000]) + "\n",
                text=True,
                check=True,
                capture_output=True,
                timeout=300,
            )
        if checkout.exists():
            common = run(
                [
                    "git",
                    "-C",
                    checkout,
                    "rev-parse",
                    "--path-format=absolute",
                    "--git-common-dir",
                ]
            ).strip()
            if Path(common).resolve() != (clone / ".git").resolve():
                raise ValueError(
                    "checkout Git directory does not belong to the experiment clone"
                )
            # Only this experiment's incomplete checkout is repaired, never a shared cache.
            run(["git", "-C", checkout, "reset", "--hard", base], 180)
        else:
            run(
                ["git", "-C", clone, "worktree", "add", "--detach", checkout, base], 180
            )
    root = checkout / entry["path"]
    graph = case / "graph-symbols.json"
    option = "--python" if entry["language"] == "python" else "--depcruise"
    program = options.python if entry["language"] == "python" else options.depcruise
    command = [
        str(binary),
        "scan",
        str(root),
        "--language",
        entry["language"],
        option,
        program,
        *entry["flags"],
        "--symbols",
        "-o",
        str(graph),
    ]
    metadata = case / "scan-symbols.json"
    if not metadata.exists():
        started = time.monotonic()
        result = subprocess.run(
            command,
            check=False,
            text=True,
            capture_output=True,
            timeout=600,
            env=dict(os.environ, BOUNDRAIL_TYPESCRIPT=options.typescript),
        )
        (case / "scan.log").write_text(result.stdout + result.stderr)
        if result.returncode:
            raise RuntimeError(f"scan failed: {case}/scan.log")
        doc = json.loads(graph.read_text())
        if doc["source"]["git"]["commit"] != base or doc["source"]["git"].get("dirty"):
            raise RuntimeError("base provenance mismatch")
        dump(
            metadata,
            {
                "command": command,
                "base": base,
                "graph_sha256": digest(graph),
                "engine_sha256": digest(binary),
                "seconds": time.monotonic() - started,
                "source": doc["source"],
            },
        )
    return case, root, json.loads(graph.read_text())


def select_repo(entry, count, binary, options):
    slug = entry["repo"].removeprefix("https://github.com/")
    pulls = []
    for page in range(1, 6):
        batch = api(
            f"repos/{slug}/pulls?state=closed&sort=updated&direction=desc&per_page=100&page={page}",
            WORK
            / "api"
            / (
                f"{entry['name']}-pulls.json"
                if page == 1
                else f"{entry['name']}-pulls-{page}.json"
            ),
        )
        pulls.extend(batch)
        if len(batch) < 100:
            break
    pulls = sorted(
        (p for p in pulls if p["merged_at"]),
        key=lambda p: (p["merged_at"], p["number"]),
        reverse=True,
    )
    chosen, decisions = [], []
    for pull in pulls:
        number = pull["number"]
        files = []
        for page in range(1, 31):
            batch = api(
                f"repos/{slug}/pulls/{number}/files?per_page=100&page={page}",
                WORK / "api" / f"{entry['name']}-{number}-files-{page}.json",
            )
            files.extend(batch)
            if len(batch) < 100:
                break
        changes = [
            {
                "path": f["filename"],
                "previous": f.get("previous_filename"),
                "status": f["status"],
            }
            for f in files
        ]
        prelim = [
            (f["previous"] if f["status"] == "renamed" else f["path"], f)
            for f in changes
            if f["status"] in ("modified", "renamed")
        ]
        prelim = [(relative_source(path, entry), f) for path, f in prelim]
        prelim = [(path, f) for path, f in prelim if path]
        decision = {
            "number": number,
            "url": pull["html_url"],
            "title": pull["title"],
            "merged_at": pull["merged_at"],
            "changes": changes,
        }
        if len(prelim) < 2:
            decision.update(
                status="ineligible",
                reason="fewer than two existing source changes within scanned root",
            )
            decisions.append(decision)
            continue
        try:
            case, root, doc = scan(entry, pull, binary, options)
            nodes = {f["path"]: f for f in doc["files"]}
            eligible = sorted(
                {
                    path
                    for path, _ in prelim
                    if path in nodes
                    and not nodes[path].get("barrel")
                    and PurePosixPath(path).name != "index.ts"
                    and nodes[path].get("class") != "tests"
                }
            )
            decision["excluded_changes"] = [
                {"path": path, "reason": "not in production base graph or pure barrel"}
                for path, _ in prelim
                if path not in eligible
            ]
            if len(eligible) < 2:
                decision.update(
                    status="ineligible",
                    reason="fewer than two non-barrel production files in base graph",
                )
            else:
                target = heldout(slug, number, eligible)
                result = dict(
                    decision,
                    status="selected",
                    name=entry["name"],
                    repo=slug,
                    base=pull["base"]["sha"],
                    head=pull["head"]["sha"],
                    merge=pull["merge_commit_sha"],
                    base_is_ancestor_of_merge=True,
                    eligible=eligible,
                    heldout=target,
                    seeds=[p for p in eligible if p != target],
                    graph=str(case / "graph-symbols.json"),
                    root=str(root),
                    scan_metadata=json.loads((case / "scan-symbols.json").read_text()),
                )
                chosen.append(result)
                decision.update(status="selected", eligible=eligible)
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
            decision.update(status="scan_failed", reason=str(error))
        decisions.append(decision)
        dump(
            WORK / "selection" / f"{entry['name']}.json",
            {"entry": entry, "decisions": decisions, "chosen": chosen},
        )
        if len(chosen) == count:
            break
    print(
        f"{entry['name']}: {len(chosen)}/{count} cases, {len(decisions)} considered",
        flush=True,
    )
    return {"entry": entry, "decisions": decisions, "chosen": chosen}


def keywords(task):
    stop = {
        "the",
        "and",
        "for",
        "with",
        "from",
        "this",
        "that",
        "into",
        "fix",
        "feat",
        "chore",
        "refactor",
    }
    return sorted(
        {
            s.lower()
            for s in re.findall(r"[A-Za-z][A-Za-z0-9_]+", task)
            if len(s) > 2 and s.lower() not in stop
        }
    )


def cost(path, reason, payload):
    return math.ceil(
        (len(path.encode("utf8")) + len(reason.encode("utf8")) + payload + 160) / 4
    )


def baseline(case, budget):
    root = Path(case["root"])
    doc = json.loads(Path(case["graph"]).read_text())
    paths = [
        f["path"]
        for f in doc["files"]
        if not f.get("barrel")
        and f.get("class") != "tests"
        and PurePosixPath(f["path"]).name != "index.ts"
    ]
    seeds = set(case["seeds"])
    folders = {str(PurePosixPath(p).parent) for p in seeds}
    words = keywords(case["title"])
    choices = []
    for path in paths:
        file = root / path
        if not file.is_file() or not file.resolve().is_relative_to(root.resolve()):
            continue
        data = file.read_bytes()
        content = path.lower() + "\n" + data.decode("utf8", errors="replace").lower()
        hits = sum(word in content for word in words)
        folder = str(PurePosixPath(path).parent)
        if path not in seeds and not hits and folder not in folders:
            continue
        distance = min(
            len(PurePosixPath(folder).parts)
            + len(PurePosixPath(f).parts)
            - 2
            * len(
                os.path.commonprefix(
                    [PurePosixPath(folder).parts, PurePosixPath(f).parts]
                )
            )
            for f in folders
        )
        reason = (
            "explicit seed"
            if path in seeds
            else "grep task keywords"
            if hits
            else "same seed folder"
        )
        choices.append(
            (path not in seeds, -hits, distance, path, cost(path, reason, len(data)))
        )
    selected, used = [], case.get("overhead_tokens", 0)
    # Share the context command's mandatory constraint files and their charged costs.
    for item in case.get("constraints", []):
        if used + item["tokens"] <= budget:
            selected.append(item["path"])
            used += item["tokens"]
    for _, _, _, path, tokens in sorted(choices):
        if used + tokens <= budget:
            selected.append(path)
            used += tokens
    return {"paths": selected, "tokens": used}


def score(cases, binary, budget):
    rows = []
    for case in cases:
        directory = WORK / "scores" / f"{case['name']}-{case['number']}" / str(budget)
        directory.mkdir(parents=True, exist_ok=True)
        if digest(Path(case["graph"])) != case["scan_metadata"]["graph_sha256"]:
            raise ValueError("frozen base graph hash changed")
        if (
            run(["git", "-C", case["root"], "rev-parse", "HEAD"]).strip()
            != case["base"]
        ):
            raise ValueError("source checkout changed from frozen base")
        if run(
            [
                "git",
                "-C",
                case["root"],
                "status",
                "--porcelain",
                "--untracked-files=all",
            ]
        ).strip():
            raise ValueError(
                "source checkout is dirty, staged or contains untracked files"
            )
        command = [
            str(binary),
            "context",
            "--graph",
            case["graph"],
            "--root",
            case["root"],
            "--task",
            case["title"],
            "--files",
            *case["seeds"],
            "--budget",
            str(budget),
            "--format",
            "json",
        ]
        try:
            report = json.loads(run(command, 180))
            dump(directory / "context.json", report)
            entries = report["entries"]
            paths = [item["path"] for item in entries]
            constrained = dict(
                case,
                overhead_tokens=report["overhead_tokens"],
                constraints=[
                    item for item in entries if item.get("tier") == "constraints"
                ],
            )
            other = baseline(constrained, budget)
            row = {
                "name": case["name"],
                "pr": case["number"],
                "url": case["url"],
                "base": case["base"],
                "heldout": case["heldout"],
                "seeds": case["seeds"],
                "context_hit": case["heldout"] in paths,
                "context_body_hit": any(
                    item["path"] == case["heldout"] and item.get("mode") == "body"
                    for item in entries
                ),
                "baseline_hit": case["heldout"] in other["paths"],
                "context_tokens": report["used_tokens"],
                "baseline_tokens": other["tokens"],
                "context_paths": paths,
                "baseline_paths": other["paths"],
                "context_command": command,
                "context_sha256": digest(directory / "context.json"),
                "status": "ok",
            }
            if row["context_tokens"] > budget or row["baseline_tokens"] > budget:
                raise ValueError("budget exceeded")
        except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            row = {
                "name": case["name"],
                "pr": case["number"],
                "status": "failed",
                "error": str(error),
            }
        rows.append(row)
    good = [r for r in rows if r["status"] == "ok"]
    return {
        "budget": budget,
        "attempted": len(rows),
        "completed": len(good),
        "context_hits": sum(r["context_hit"] for r in good),
        "context_body_hits": sum(r["context_body_hit"] for r in good),
        "baseline_hits": sum(r["baseline_hit"] for r in good),
        "context_only": sum(r["context_hit"] and not r["baseline_hit"] for r in good),
        "baseline_only": sum(r["baseline_hit"] and not r["context_hit"] for r in good),
        "denominator": len(good),
        "cases": rows,
    }


def results_markdown(result, frozen):
    lines = [
        "## Results",
        "",
        "One held-out existing production file per PR; export-name summaries count as discovery, not full-body reading or complete function signatures.",
        "",
        "| Token budget | Context discovery | Context full body | Grep + folder | Context only / baseline only | Mean tokens (context / baseline) |",
        "| ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for row in result["results"]:
        count = row["denominator"]
        good = [r for r in row["cases"] if r["status"] == "ok"]
        ratios = [
            f"{row[key]}/{count} ({100 * row[key] / count:.1f}%)" if count else "0/0"
            for key in ["context_hits", "context_body_hits", "baseline_hits"]
        ]
        means = [
            round(sum(r[key] for r in good) / count) if count else 0
            for key in ["context_tokens", "baseline_tokens"]
        ]
        lines.append(
            f"| {row['budget']:,} | {' | '.join(ratios)} | {row['context_only']} / {row['baseline_only']} | {means[0]:,} / {means[1]:,} |"
        )
    primary = next(r for r in result["results"] if r["budget"] == 40000)
    lines += [
        "",
        f"At the primary 40k budget, context discovered {primary['context_hits']} held-out files versus {primary['baseline_hits']} for the baseline. Context supplied full bodies for {primary['context_body_hits']}; {primary['context_hits'] - primary['context_body_hits']} discoveries supplied export names only. The paired comparison has {primary['context_only']} context-only hits and {primary['baseline_only']} baseline-only hits. This small convenience sample does not establish general superiority.",
        "",
        "Greedy packing can be nonmonotonic: a larger budget admits an earlier large file, which can displace a later held-out file. Report every budget rather than choosing the most favorable one.",
    ]
    selected = frozen["cases"]
    changed = sum(len(c["changes"]) for c in selected)
    eligible = sum(len(c["eligible"]) for c in selected)
    added = sum(f["status"] == "added" for c in selected for f in c["changes"])
    deleted = sum(f["status"] == "removed" for c in selected for f in c["changes"])
    lines += [
        "",
        f"The 30 selected PRs changed {changed:,} paths: {eligible:,} were eligible existing production files, {added:,} were added, {deleted:,} removed; all remaining paths failed source/root/default-exclusion/barrel eligibility. Full reasons and all considered PRs are in [the frozen inventory](context-prs.json). Base graphs include the surface index, with no hotspot or co-change attachments; historical ranking boosts were not measured.",
        "",
        f"Measured source and executed harness commit: `{result.get('runner_commit', result.get('engine_commit', 'unspecified'))}`. The executed harness hash belongs to that commit; subsequent report wording changes do not rerun or alter rankings. The [machine-readable results](results/context-pruning.json) retain every selected path, token count and raw report hash. Source graphs, API responses and unabridged context reports are archived under the local experiment directory, with immutable commit IDs and commands recorded for reproduction.",
        "",
        "| Repository / PR | Base | Held-out file | Context at 40k | Grep + folder |",
        "| --- | --- | --- | --- | --- |",
    ]
    primary = next(r for r in result["results"] if r["budget"] == 40000)
    for row in primary["cases"]:
        case = next(
            c for c in selected if c["name"] == row["name"] and c["number"] == row["pr"]
        )
        if row["status"] != "ok":
            context, other = "failed", "failed"
        else:
            context = (
                "body"
                if row["context_body_hit"]
                else "exports"
                if row["context_hit"]
                else "miss"
            )
            other = "body" if row["baseline_hit"] else "miss"
        lines.append(
            f"| [{case['name']} #{case['number']}]({case['url']}) | [`{case['base'][:8]}`](https://github.com/{case['repo']}/commit/{case['base']}) | `{case['heldout']}` | {context} | {other} |"
        )
    return "\n".join(lines) + "\n"


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("mode", choices=["select", "score"])
    p.add_argument("--binary", type=Path, default=REPO / "target/release/boundrail")
    p.add_argument(
        "--depcruise",
        default="/private/tmp/boundrail-impact-collectors/node_modules/.bin/depcruise",
    )
    p.add_argument(
        "--python", default="/private/tmp/boundrail-impact-python/bin/python"
    )
    p.add_argument(
        "--typescript",
        default="/private/tmp/boundrail-impact-collectors/node_modules/typescript/package.json",
    )
    p.add_argument("--jobs", type=int, default=3)
    p.add_argument("--cases", type=Path, default=REPO / "benchmarks/context-prs.json")
    p.add_argument(
        "--output", type=Path, default=REPO / "benchmarks/results/context-pruning.json"
    )
    args = p.parse_args()
    if args.mode == "select":
        entries = json.loads((REPO / "showcase/repos.json").read_text())[:20]
        WORK.mkdir(parents=True, exist_ok=True)
        dump(WORK / "protocol.json", PROTOCOL)
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            selected = list(
                pool.map(
                    lambda pair: select_repo(
                        pair[1], 2 if pair[0] < 10 else 1, args.binary, args
                    ),
                    enumerate(entries),
                )
            )
        cases = [c for repo in selected for c in repo["chosen"]]
        dump(
            args.cases,
            {
                "protocol": PROTOCOL,
                "sample_frozen_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "manifest_sha256": digest(REPO / "showcase/repos.json"),
                "tools": {
                    "git": run(["git", "--version"]).strip(),
                    "gh": run(["gh", "--version"]).splitlines()[0],
                    "typescript": json.loads(Path(args.typescript).read_text())[
                        "version"
                    ],
                },
                "cases": cases,
                "selection": selected,
                "preparation_failures": json.loads(
                    (WORK / "preparation-failures.json").read_text()
                )
                if (WORK / "preparation-failures.json").exists()
                else [],
            },
        )
        if len(cases) != 30:
            raise SystemExit(
                f"Only {len(cases)}/30 eligible cases; see recorded failures"
            )
    else:
        frozen = json.loads(args.cases.read_text())
        if len(frozen["cases"]) != 30:
            raise SystemExit("Expected 30 frozen cases")
        results = [
            score(frozen["cases"], args.binary, b) for b in [10000, 20000, 40000]
        ]
        result = {
            "protocol": PROTOCOL,
            "cases_sha256": digest(args.cases),
            "runner_sha256": digest(Path(__file__)),
            "runner_commit": run(["git", "-C", REPO, "rev-parse", "HEAD"]).strip(),
            "engine_sha256": digest(args.binary),
            "engine_commit": run(["git", "-C", REPO, "rev-parse", "HEAD"]).strip(),
            "engine_source_dirty": bool(
                run(
                    [
                        "git",
                        "-C",
                        REPO,
                        "status",
                        "--porcelain",
                        "--untracked-files=all",
                        "--",
                        "src",
                        "Cargo.toml",
                        "Cargo.lock",
                    ]
                ).strip()
            ),
            "results": results,
        }
        dump(args.output, result)
        document = REPO / "benchmarks/CONTEXT.md"
        text = document.read_text().split("\n<!-- RESULTS -->")[0]
        text = text.replace(
            "Results will be added only after all 30 observations run.",
            "Results below use the frozen 30-PR sample and unchanged ranking parameters.",
        )
        document.write_text(
            text.rstrip() + "\n\n<!-- RESULTS -->\n" + results_markdown(result, frozen)
        )
        for r in results:
            print({k: v for k, v in r.items() if k != "cases"})


if __name__ == "__main__":
    main()
