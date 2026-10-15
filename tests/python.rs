mod common;

use common::scan_recorded;
use serde_json::{Value, json};

#[test]
fn py_cycles_mapping_barrels_and_cycle() {
    // No --language: the fixture holds only .py files, so detection picks python.
    let (doc, _, _) = scan_recorded("py-cycles", &[]);
    assert_eq!(doc["source"]["language"], "python");
    assert_eq!(
        doc["source"]["collector"],
        json!({ "name": "grimp", "version": "3.17", "cruised": 12 })
    );
    assert_eq!(doc["config"]["source_root"], "mypkg");
    // api/__init__.py only imports its own submodules: a barrel. core/__init__.py imports
    // db.session and defines code: ordinary. mypkg/tests is excluded as test code.
    assert_eq!(
        doc["files"],
        json!([
            { "path": "mypkg/__init__.py", "module": ".", "barrel": false },
            { "path": "mypkg/api/__init__.py", "module": "api", "barrel": true },
            { "path": "mypkg/api/handlers.py", "module": "api", "barrel": false },
            { "path": "mypkg/api/schemas.py", "module": "api", "barrel": false },
            { "path": "mypkg/cli.py", "module": ".", "barrel": false },
            { "path": "mypkg/core/__init__.py", "module": "core", "barrel": false },
            { "path": "mypkg/core/service.py", "module": "core", "barrel": false },
            { "path": "mypkg/db/__init__.py", "module": "db", "barrel": false },
            { "path": "mypkg/db/repo.py", "module": "db", "barrel": false },
            { "path": "mypkg/db/session.py", "module": "db", "barrel": false }
        ])
    );
    assert_eq!(
        doc["edges"],
        json!([
            { "from": "mypkg/api/__init__.py", "to": "mypkg/api/handlers.py", "kind": "static", "verdict": null, "rule": null },
            { "from": "mypkg/api/__init__.py", "to": "mypkg/api/schemas.py", "kind": "static", "verdict": null, "rule": null },
            { "from": "mypkg/api/handlers.py", "to": "mypkg/core/service.py", "kind": "static", "verdict": null, "rule": null },
            { "from": "mypkg/cli.py", "to": "mypkg/api/__init__.py", "kind": "static", "verdict": null, "rule": null },
            { "from": "mypkg/core/__init__.py", "to": "mypkg/db/session.py", "kind": "static", "verdict": null, "rule": null },
            { "from": "mypkg/core/service.py", "to": "mypkg/db/repo.py", "kind": "static", "verdict": null, "rule": null },
            { "from": "mypkg/db/repo.py", "to": "mypkg/core/__init__.py", "kind": "static", "verdict": null, "rule": null },
            { "from": "mypkg/db/repo.py", "to": "mypkg/db/session.py", "kind": "static", "verdict": null, "rule": null }
        ])
    );
    // cli -> api/__init__ counts as cli -> handlers and cli -> schemas.
    assert_eq!(
        doc["modules"],
        json!([
            { "id": ".", "files": 2, "ca": 0, "ce": 1, "instability": 1.0,
              "internal_edges": 0, "outgoing_edges": 2, "cohesion": 0.0 },
            { "id": "api", "files": 3, "ca": 1, "ce": 1, "instability": 0.5,
              "internal_edges": 0, "outgoing_edges": 1, "cohesion": 0.0 },
            { "id": "core", "files": 2, "ca": 2, "ce": 1, "instability": 0.3333,
              "internal_edges": 0, "outgoing_edges": 2, "cohesion": 0.0 },
            { "id": "db", "files": 3, "ca": 1, "ce": 1, "instability": 0.5,
              "internal_edges": 1, "outgoing_edges": 1, "cohesion": 0.5 }
        ])
    );
    assert_eq!(
        doc["module_edges"],
        json!([
            { "from": ".", "to": "api", "imports": 1, "forbidden": 0 },
            { "from": "api", "to": "core", "imports": 1, "forbidden": 0 },
            { "from": "core", "to": "db", "imports": 2, "forbidden": 0 },
            { "from": "db", "to": "core", "imports": 1, "forbidden": 0 }
        ])
    );
    // core -> db carries 2 imports, db -> core 1. No file cycle: service -> repo -> core/__init__
    // never returns to service.
    assert_eq!(
        doc["cycles"],
        json!([
            { "level": "module", "members": ["core", "db"], "exact": true,
              "break": [{ "from": "mypkg/db/repo.py", "to": "mypkg/core/__init__.py" }] }
        ])
    );

    let findings = doc["findings"].as_array().unwrap();
    let kinds: Vec<(&str, &str)> = findings
        .iter()
        .map(|f| (f["kind"].as_str().unwrap(), f["subject"].as_str().unwrap()))
        .collect();
    // core/__init__.py also talks mostly to db, but a package's __init__ cannot move alone.
    // cli.py talks only to api (via its barrel), but nothing imports it: an entry point.
    assert_eq!(
        kinds,
        [
            ("cycle", "core, db"),
            ("misplaced-file", "mypkg/db/repo.py"),
            ("seam", "."),
            ("small-repo", "."),
        ]
    );
    assert_eq!(findings[0]["evidence"], doc["cycles"][0]);
    assert_eq!(
        findings[1]["evidence"],
        json!({ "own_module_edges": 1, "target_module": "core", "target_module_edges": 2,
                "via_barrel": false })
    );
    // repo.py into core removes db -> core, the only edge back. api and . do not change.
    assert_eq!(
        findings[1]["suggestion"]["move"]["to"],
        "mypkg/core/repo.py"
    );
    assert_eq!(doc["summary"]["cycles"], 1);
    assert_eq!(
        findings[1]["simulation"]["after"],
        json!({ "forbidden": 0, "unplanned": 0, "cycles": 0, "modules": {
            "core": { "cohesion": 0.5, "ca": 1, "ce": 1, "instability": 0.5 },
            "db": { "cohesion": Value::Null, "ca": 1, "ce": 0, "instability": 0.0 } } })
    );
    // repo.py has degree 3, above the 90th percentile (2), so it is a hub and left out.
    // Q(clusters) = 3/4 - (6/8)^2 + 1/4 - (2/8)^2; Q(folders) = -18/64. The clusters hold 6 of
    // the 9 non-barrel files, at least half.
    assert_eq!(
        findings[2]["evidence"],
        json!({ "modularity": 0.375, "folder_modularity": -0.2813,
                "hubs": ["mypkg/db/repo.py"], "via_barrel": true, "clusters": [
                    { "files": ["mypkg/api/handlers.py", "mypkg/api/schemas.py", "mypkg/cli.py",
                                "mypkg/core/service.py"], "modules": [".", "api", "core"] },
                    { "files": ["mypkg/core/__init__.py", "mypkg/db/session.py"],
                      "modules": ["core", "db"] } ] })
    );
    assert_eq!(
        findings[3]["evidence"],
        json!({ "files": 10, "modules": 4 })
    );
}

#[test]
fn python_package_detection_finds_namespace_packages_and_skips_tests() {
    use archgraph::collect::{DEFAULT_EXCLUDES, TEST_EXCLUDES, globset, grimp};
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("namespace-package");
    let _ = std::fs::remove_dir_all(&root);
    // app has no __init__.py; tests has one but is excluded; docs holds no Python.
    for (file, text) in [
        ("app/service.py", "x = 1\n"),
        ("tests/__init__.py", ""),
        ("tests/test_service.py", "import app.service\n"),
        ("docs/index.md", "# docs\n"),
    ] {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let defaults: Vec<String> = DEFAULT_EXCLUDES
        .iter()
        .chain(TEST_EXCLUDES)
        .map(|g| g.to_string())
        .collect();
    let excludes = globset(&defaults).unwrap();
    assert_eq!(grimp::package(&root, ".", None, &excludes).unwrap(), "app");

    // With tests kept, two packages: ambiguous.
    let err = grimp::package(&root, ".", None, &globset(&[]).unwrap()).unwrap_err();
    assert!(err.to_string().contains("app, tests"), "{err}");
}

#[test]
fn namespace_packages_are_not_files() {
    // grimp lists the namespace packages `app` and `app.sub` (no __init__.py) as modules.
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("namespace-scan");
    let _ = std::fs::remove_dir_all(&root);
    for file in ["app/a.py", "app/b.py", "app/sub/c.py"] {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    let dump = json!({ "grimp_version": "3.17",
        "modules": ["app", "app.a", "app.b", "app.sub", "app.sub.c"],
        "imports": [ { "importer": "app.a", "imported": "app.b" },
                     { "importer": "app.sub.c", "imported": "app.a" },
                     { "importer": "app.sub.c", "imported": "app.sub" } ] });
    let input = common::write_tmp("namespace-scan.json", &dump.to_string());
    let (doc, _, _) = common::scan(&[root.to_str().unwrap(), "--collector-input", &input]);
    let paths: Vec<&str> = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["app/a.py", "app/b.py", "app/sub/c.py"]);
    assert_eq!(doc["edges"].as_array().unwrap().len(), 2);
    assert_eq!(doc["source"]["collector"]["cruised"], 3);
}
