#!/usr/bin/env bash
# Host Python drives a shell-only fake binary; collector executables are forbidden sentinels.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import os
from pathlib import Path
import subprocess
import tempfile

with tempfile.TemporaryDirectory(prefix="archgraph-native-action-") as temp:
    root = Path(temp)
    project = root / "project"
    project.mkdir()
    for marker in ("package.json", "tsconfig.json", "pyproject.toml", "go.mod", "go.work", "mix.exs"):
        (project / marker).write_text("")
    forbidden = root / "forbidden-tools"
    forbidden.mkdir()
    for tool in ("node", "npm", "npx", "python", "python3", "pip", "depcruise", "go", "cargo", "rustc", "rust-analyzer", "java", "javac", "elixir", "curl"):
        path = forbidden / tool
        path.write_text('#!/bin/sh\nprintf "%s\\n" "$0" >> "$CALLED"\nexit 99\n')
        path.chmod(0o755)
    binary = root / "archgraph"
    binary.write_text('''#!/bin/bash
set -eu
if [[ $1 == --version ]]; then echo 'archgraph test'; exit; fi
for arg in "$@"; do
  if [[ ${REJECT_ENGINE:-} == 1 && $arg == --engine ]]; then exit 88; fi
done
printf '%s\\n' "$@" > "$CAPTURE"
while (($#)); do
  if [[ $1 == --out-dir ]]; then
    mkdir -p "$2"
    printf '%s\\n' '{"summary":{"forbidden":0}}' > "$2/archgraph.json"
    break
  fi
  shift
done
echo 'native report'
''')
    binary.chmod(0o755)
    runner = root / "runner"
    runner.mkdir()
    env = dict(os.environ, PATH=str(forbidden) + os.pathsep + os.environ["PATH"],
               INPUT_BINARY=str(binary), INPUT_PATH=str(project), INPUT_ENGINE="native",
               INPUT_BASE_REF="", INPUT_RULES="", INPUT_PACKAGE="", INPUT_EXCLUDE="",
               RUNNER_TEMP=str(runner), GITHUB_OUTPUT=str(root / "output"),
               GITHUB_STEP_SUMMARY=str(root / "summary"),
               CAPTURE=str(root / "capture"), CALLED=str(root / "called"))
    for language in ("typescript", "python", "go", "rust", "java", "elixir", ""):
        env["INPUT_LANGUAGE"] = language
        result = subprocess.run(["bash", "ci/action.sh"], env=env, capture_output=True, text=True)
        assert result.returncode == 0, result.stdout + result.stderr
        args = (root / "capture").read_text().splitlines()
        assert args[args.index("--engine") + 1] == "native", args
        if language:
            assert args.index("--engine") < args.index("--language"), args
        assert not any(arg in args for arg in ("--depcruise", "--python", "--go", "--elixir")), args
        assert not (root / "called").exists(), "native Action invoked an external collector/tool"
    env["INPUT_LANGUAGE"] = "go"
    env["ARCHGRAPH_GO"] = str(root / "explicit-go")
    result = subprocess.run(["bash", "ci/action.sh"], env=env, capture_output=True, text=True)
    assert result.returncode == 0, result.stdout + result.stderr
    args = (root / "capture").read_text().splitlines()
    assert args[args.index("--go") + 1] == env["ARCHGRAPH_GO"], args
    assert not (root / "called").exists()
    env.update(INPUT_ENGINE="reference", INPUT_LANGUAGE="elixir", REJECT_ENGINE="1",
               ARCHGRAPH_ELIXIR=str(forbidden / "elixir"))
    result = subprocess.run(["bash", "ci/action.sh"], env=env, capture_output=True, text=True)
    assert result.returncode == 0, result.stdout + result.stderr
    assert "--engine" not in (root / "capture").read_text().splitlines()
    env["INPUT_ENGINE"] = "unknown"
    result = subprocess.run(["bash", "ci/action.sh"], env=env, capture_output=True, text=True)
    assert result.returncode == 2 and "engine must be" in result.stdout, result.stdout

print("Native Action forwards its engine, skips external setup and rejects unknown engines")
PY
