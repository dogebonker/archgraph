mod common;

use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn native(args: &[&str]) -> Value {
    let out = common::archgraph(args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value = serde_json::from_slice(&out.stdout).unwrap();
    common::validate(&value);
    value
}

#[test]
fn native_scan_and_symbols_need_no_external_collectors() {
    let file = common::write_tmp("native-cli-ts/src/core/model.ts", "export class Model {}\n");
    let root = Path::new(&file).ancestors().nth(3).unwrap();
    common::write_tmp(
        "native-cli-ts/src/core/index.ts",
        "export { Model } from './model';\n",
    );
    common::write_tmp(
        "native-cli-ts/src/app/main.ts",
        "import { Model } from '../core';\nexport const model = new Model();\nconsole.log(model);\n",
    );
    let doc = native(&[
        "scan",
        root.to_str().unwrap(),
        "--engine",
        "native",
        "--language",
        "typescript",
        "--symbols",
        "--depcruise",
        "/nonexistent/native-must-not-execute",
        "--python",
        "/nonexistent/python",
    ]);
    assert_eq!(doc["source"]["collector"]["name"], "tree-sitter-typescript");
    assert_eq!(doc["config"]["collection"]["engine"], "native");
    assert!(
        doc["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["from"] == "src/app/main.ts"
                && e["to"] == "src/core/model.ts"
                && e["original_to"] == "src/core/index.ts")
    );
    assert!(!doc["symbols"]["exports"].as_array().unwrap().is_empty());
    assert_eq!(
        doc["globals"],
        serde_json::json!([{"from": "src/app/main.ts", "name": "console", "line": 3}])
    );
    let graph = common::write_tmp(
        "native-cli-ts/graph.json",
        &serde_json::to_string(&doc).unwrap(),
    );
    let checked = common::archgraph(&["render", &graph, "-o", "/dev/null"]);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );

    let file = common::write_tmp(
        "native-cli-py/pkg/__init__.py",
        "raise RuntimeError('must never execute')\n",
    );
    let root = Path::new(&file).ancestors().nth(2).unwrap();
    common::write_tmp("native-cli-py/pkg/models.py", "class Model: pass\n");
    common::write_tmp("native-cli-py/pkg/main.py", "from .models import Model\n");
    let doc = native(&[
        "scan",
        root.to_str().unwrap(),
        "--engine",
        "native",
        "--language",
        "python",
        "--package",
        "pkg",
        "--symbols",
        "--python",
        "/nonexistent/python",
    ]);
    assert_eq!(doc["source"]["collector"]["name"], "tree-sitter-python");
    assert_eq!(doc["config"]["source_root"], "pkg");
    assert!(
        doc["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["from"] == "pkg/main.py" && e["to"] == "pkg/models.py")
    );
    assert!(doc.get("globals").is_none());
}

#[test]
fn native_history_keeps_engine_and_skips_interpreter_version_probes() {
    let file = common::write_tmp("native-cli-history/pkg/__init__.py", "");
    let root = Path::new(&file).ancestors().nth(2).unwrap();
    common::write_tmp("native-cli-history/pkg/main.py", "from . import core\n");
    common::write_tmp("native-cli-history/pkg/core.py", "answer = 1\n");
    for args in [
        &["init", "-q"][..],
        &["add", "."][..],
        &["commit", "--allow-empty", "-qm", "first"][..],
    ] {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=archgraph",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let output = root.join("history-output");
    let out = common::archgraph(&[
        "history",
        root.to_str().unwrap(),
        "--engine",
        "native",
        "--language",
        "python",
        "--package",
        "pkg",
        "--python",
        "/nonexistent/python",
        "--sample",
        "commits",
        "--limit",
        "1",
        "--out-dir",
        output.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let history: Value =
        serde_json::from_slice(&std::fs::read(output.join("history.json")).unwrap()).unwrap();
    common::validate(&history);
    assert_eq!(history["snapshots"].as_array().unwrap().len(), 1);
}

#[test]
fn native_compiler_languages_preserve_limits_without_symbol_output() {
    let rust_file = common::write_tmp(
        "native-cli-rust/src/lib.rs",
        "mod model; pub use model::Model;\n",
    );
    let rust_root = Path::new(&rust_file).ancestors().nth(2).unwrap();
    common::write_tmp("native-cli-rust/src/model.rs", "pub struct Model;\n");
    common::write_tmp(
        "native-cli-rust/Cargo.toml",
        "[package]\nname='native-cli'\nversion='0.1.0'\n",
    );
    for (language, fixture, flag) in [
        ("java", common::fixture("java-resolve"), "--java"),
        ("rust", rust_root.to_path_buf(), "--rust-analyzer"),
        ("elixir", common::fixture("elixir-basic"), "--elixir"),
    ] {
        let doc = native(&[
            "scan",
            fixture.to_str().unwrap(),
            "--language",
            language,
            "--engine",
            "native",
            flag,
            "/nonexistent/native-must-not-execute",
        ]);
        assert_eq!(
            doc["source"]["collector"]["name"],
            format!("tree-sitter-{language}")
        );
        assert!(!doc["edges"].as_array().unwrap().is_empty());
        assert!(doc.get("symbols").is_none());
        assert!(
            !doc["source"]["collector"]["limitations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
