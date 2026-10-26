#!/usr/bin/env bash
# Exercise action argument forwarding and plain-text summaries without installing a runtime.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import json
import os
from pathlib import Path
import subprocess
import tempfile

with tempfile.TemporaryDirectory(prefix="boundrail-elixir-action-") as temp:
    root = Path(temp)
    project = root / "project"
    project.mkdir()
    (project / "mix.exs").write_text('raise "must not execute"\n')
    binary = root / "boundrail"
    binary.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
if "--version" in sys.argv:
    print("boundrail test")
elif sys.argv[1] == "scan":
    pathlib.Path(os.environ["BASE_SCAN"]).write_text(json.dumps(os.listdir(sys.argv[2])))
    pathlib.Path(sys.argv[sys.argv.index("-o") + 1]).write_text('{"summary":{"forbidden":0}}')
elif sys.argv[1] == "diff":
    print("no changes" if "--format" in sys.argv else '{"new_forbidden":[],"new_cycles":[]}')
else:
    pathlib.Path(os.environ["CAPTURE"]).write_text(json.dumps(sys.argv[1:]))
    output = pathlib.Path(sys.argv[sys.argv.index("--out-dir") + 1])
    output.mkdir(parents=True)
    (output / "boundrail.json").write_text('{"summary":{"forbidden":0}}')
    print(os.environ.get("REPORT_TEXT", "plain report without code fences"))
''')
    binary.chmod(0o755)
    runtime = root / "elixir fake"
    runtime.write_text("#!/bin/sh\nexit 0\n")
    runtime.chmod(0o755)
    runner = root / "runner"
    runner.mkdir()
    env = dict(os.environ, INPUT_BINARY=str(binary), INPUT_PATH=str(project),
               INPUT_LANGUAGE="elixir", INPUT_BASE_REF="", INPUT_RULES="",
               RUNNER_TEMP=str(runner), GITHUB_OUTPUT=str(root / "output"),
               GITHUB_STEP_SUMMARY=str(root / "summary"),
               CAPTURE=str(root / "capture"), BOUNDRAIL_ELIXIR=str(runtime))

    def run(expected):
        result = subprocess.run(["bash", "ci/action.sh"], env=env,
                                capture_output=True, text=True)
        assert result.returncode == expected, result.stdout + result.stderr
        return result

    run(0)
    arguments = json.loads((root / "capture").read_text())
    assert arguments[arguments.index("--elixir") + 1] == str(runtime)
    # Outside git: no co-change and no note.
    assert "--cochange" not in arguments
    assert "Co-change" not in (root / "summary").read_text()
    assert "```text\nplain report without code fences\n```" in (root / "summary").read_text()

    env["REPORT_TEXT"] = "repository controls ````` these backticks"
    (root / "summary").write_text("")
    run(0)
    assert "``````text\n" in (root / "summary").read_text()

    env["BOUNDRAIL_ELIXIR"] = str(root / "missing-elixir")
    assert "Elixir scans need" in run(2).stdout

    env["INPUT_LANGUAGE"] = ""
    run(0)
    arguments = json.loads((root / "capture").read_text())
    assert "--language" not in arguments
    assert arguments[arguments.index("--elixir") + 1] == env["BOUNDRAIL_ELIXIR"]

    # A pull request that deletes the rules file; the base scan sees the project's node_modules.
    repo = root / "repo"
    (repo / "node_modules").mkdir(parents=True)
    (repo / ".gitignore").write_text("node_modules/\n")
    (repo / "mix.exs").write_text("")
    (repo / "layout.yaml").write_text("version: 1\n")
    git = ["git", "-C", str(repo), "-c", "user.name=ci", "-c", "user.email=ci@example.invalid",
           "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"]
    subprocess.run(git + ["init", "-q"], check=True)
    subprocess.run(git + ["add", "-A"], check=True)
    subprocess.run(git + ["commit", "-qm", "base"], check=True)
    base = subprocess.run(git + ["rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
    subprocess.run(git + ["rm", "-q", "layout.yaml"], check=True)
    subprocess.run(git + ["commit", "-qm", "head"], check=True)
    env.update(INPUT_PATH=str(repo), INPUT_BASE_REF=base, BASE_SCAN=str(root / "base-scan"))
    (root / "summary").write_text("")
    run(0)
    summary = (root / "summary").read_text()
    assert "deletes the rules file `layout.yaml`" in summary, summary
    assert "| forbidden imports |" not in summary, summary
    assert "node_modules" in json.loads((root / "base-scan").read_text())
    # A full checkout reads co-change; a shallow one says why not.
    assert "--cochange" in json.loads((root / "capture").read_text())
    shallow = root / "shallow"
    subprocess.run(["git", "clone", "-q", "--depth=1", repo.as_uri(), str(shallow)], check=True)
    env.update(INPUT_PATH=str(shallow), INPUT_BASE_REF="")
    (root / "summary").write_text("")
    run(0)
    assert "--cochange" not in json.loads((root / "capture").read_text())
    summary = (root / "summary").read_text()
    assert "\nCo-change not read: shallow checkout; check out with fetch-depth: 0.\n" in summary, summary

print("Elixir action forwarding, runtime errors, detection, summary fences, base comparisons and co-change passed")
PY
