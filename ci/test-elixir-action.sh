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

with tempfile.TemporaryDirectory(prefix="archgraph-elixir-action-") as temp:
    root = Path(temp)
    project = root / "project"
    project.mkdir()
    (project / "mix.exs").write_text('raise "must not execute"\n')
    binary = root / "archgraph"
    binary.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
if "--version" in sys.argv:
    print("archgraph test")
else:
    pathlib.Path(os.environ["CAPTURE"]).write_text(json.dumps(sys.argv[1:]))
    output = pathlib.Path(sys.argv[sys.argv.index("--out-dir") + 1])
    output.mkdir(parents=True)
    (output / "archgraph.json").write_text('{"summary":{"forbidden":0}}')
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
               CAPTURE=str(root / "capture"), ARCHGRAPH_ELIXIR=str(runtime))

    def run(expected):
        result = subprocess.run(["bash", "ci/action.sh"], env=env,
                                capture_output=True, text=True)
        assert result.returncode == expected, result.stdout + result.stderr
        return result

    run(0)
    arguments = json.loads((root / "capture").read_text())
    assert arguments[arguments.index("--elixir") + 1] == str(runtime)
    assert "```text\nplain report without code fences\n```" in (root / "summary").read_text()

    env["REPORT_TEXT"] = "repository controls ````` these backticks"
    (root / "summary").write_text("")
    run(0)
    assert "``````text\n" in (root / "summary").read_text()

    env["ARCHGRAPH_ELIXIR"] = str(root / "missing-elixir")
    assert "Elixir scans need" in run(2).stdout

    env["INPUT_LANGUAGE"] = ""
    run(0)
    arguments = json.loads((root / "capture").read_text())
    assert "--language" not in arguments
    assert arguments[arguments.index("--elixir") + 1] == env["ARCHGRAPH_ELIXIR"]

print("Elixir action forwarding, runtime errors, detection and summary fences passed")
PY
