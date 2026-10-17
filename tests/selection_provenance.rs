//! Historical probes must reproduce included support/generated files, not promote defaults to
//! explicit exclusions which would override those includes.
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
#[cfg(unix)]
fn bisect_retains_layout_included_examples_in_every_probe() {
    use std::os::unix::fs::PermissionsExt;
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("selection-provenance-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.join("repo");
    fs::create_dir_all(root.join("pkg")).unwrap();
    fs::create_dir_all(root.join("examples")).unwrap();
    git(&root, &["init", "-q"]);
    fs::write(root.join("pkg/__init__.py"), "").unwrap();
    fs::write(root.join("pkg/a.py"), "value = 1\n").unwrap();
    fs::write(root.join("examples/one.py"), "from pkg import a\n").unwrap();
    fs::write(
        root.join("layout.yaml"),
        "version: 1\ninclude: ['examples/**']\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "before cycle"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    fs::write(root.join("pkg/a.py"), "from examples import one\n").unwrap();
    git(&root, &["add", "."]);
    git(
        &root,
        &["commit", "-qm", "close cycle through included example"],
    );
    let head = git(&root, &["rev-parse", "HEAD"]);

    // A live collector transport for deterministic fixtures: honor each historical source tree.
    let python = directory.join("python");
    fs::write(&python, r#"#!/usr/bin/env python3
import json,pathlib,sys
root = pathlib.Path(sys.argv[7])
imports = [{'importer':'examples.one','imported':'pkg.a','kind':'runtime'}]
if 'examples' in (root/'pkg/a.py').read_text():
    imports.append({'importer':'pkg.a','imported':'examples.one','kind':'runtime'})
print(json.dumps({'grimp_version':'fixture','modules':['pkg','pkg.a','examples.one'],'imports':imports}))
"#).unwrap();
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
    let graph = directory.join("graph.json");
    let scan = Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .arg("scan")
        .arg(&root)
        .args(["--language", "python", "--package", "pkg", "--python"])
        .arg(&python)
        .arg("--rules")
        .arg(root.join("layout.yaml"))
        .arg("-o")
        .arg(&graph)
        .output()
        .unwrap();
    assert!(
        scan.status.success(),
        "{}",
        String::from_utf8_lossy(&scan.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&fs::read(&graph).unwrap()).unwrap();
    assert_eq!(doc["cycles"].as_array().unwrap().len(), 1);
    assert!(
        doc["config"]["explicit_excludes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let investigation = Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .arg("bisect")
        .arg(&graph)
        .arg("--root")
        .arg(&root)
        .arg("--from")
        .arg(&base)
        .arg("--python")
        .arg(&python)
        .output()
        .unwrap();
    assert!(
        investigation.status.success(),
        "{}",
        String::from_utf8_lossy(&investigation.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&investigation.stdout).unwrap();
    assert_eq!(result["introduced_by"]["commit"], head, "{result}");
    assert_eq!(result["status"], "verified-transition", "{result}");
    fs::remove_dir_all(directory).unwrap();
}
