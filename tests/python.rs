mod common;

use common::scan_recorded;
use serde_json::json;

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
    // db.session and defines code: ordinary. mypkg/tests is excluded as test code. Each file
    // directly in the source root is its own module.
    assert_eq!(
        doc["files"],
        json!([
            { "path": "mypkg/__init__.py", "module": "__init__.py", "barrel": false },
            { "path": "mypkg/api/__init__.py", "module": "api", "barrel": true },
            { "path": "mypkg/api/handlers.py", "module": "api", "barrel": false },
            { "path": "mypkg/api/schemas.py", "module": "api", "barrel": false },
            { "path": "mypkg/cli.py", "module": "cli.py", "barrel": false },
            { "path": "mypkg/core/__init__.py", "module": "core", "barrel": false },
            { "path": "mypkg/core/service.py", "module": "core", "barrel": false },
            { "path": "mypkg/db/__init__.py", "module": "db", "barrel": false },
            { "path": "mypkg/db/repo.py", "module": "db", "barrel": false },
            { "path": "mypkg/db/session.py", "module": "db", "barrel": false }
        ])
    );
    assert_eq!(
        common::without_import_metadata(&doc["edges"]),
        json!([
            { "from": "mypkg/api/__init__.py", "to": "mypkg/api/handlers.py", "kind": "reexport", "weight": 0.0, "is_reexport": true, "verdict": null, "rule": null },
            { "from": "mypkg/api/__init__.py", "to": "mypkg/api/schemas.py", "kind": "reexport", "weight": 0.0, "is_reexport": true, "verdict": null, "rule": null },
            { "from": "mypkg/api/handlers.py", "to": "mypkg/core/service.py", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "mypkg/cli.py", "to": "mypkg/api/__init__.py", "kind": "reexport", "weight": 0.0, "verdict": null, "rule": null },
            { "from": "mypkg/core/__init__.py", "to": "mypkg/db/session.py", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "mypkg/core/service.py", "to": "mypkg/db/repo.py", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "mypkg/db/repo.py", "to": "mypkg/core/__init__.py", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "mypkg/db/repo.py", "to": "mypkg/db/session.py", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null }
        ])
    );
    let namespace = doc["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| edge["from"] == "mypkg/cli.py")
        .unwrap();
    assert_eq!(namespace["confidence"], "low");
    assert_eq!(
        namespace["resolution_reason"],
        "namespace or star import cannot be narrowed"
    );
    // cli imports an un-narrowed package namespace: it remains a low-confidence structural
    // edge with zero runtime weight. Integer raw counts retain legacy barrel expansion.
    assert_eq!(
        doc["modules"],
        json!([
            { "id": "__init__.py", "files": 1, "ca": 0, "ce": 0, "instability": null,
              "internal_edges": 0, "outgoing_edges": 0, "cohesion": null, "outgoing_to": {}, "weighted_ca": 0.0, "weighted_ce": 0.0, "weighted_internal_edges": 0.0, "weighted_outgoing_edges": 0.0, "weighted_outgoing_to": {} },
            { "id": "api", "files": 3, "ca": 1, "ce": 1, "instability": 1.0,
              "internal_edges": 0, "outgoing_edges": 1, "cohesion": 0.0,
              "outgoing_to": { "core": 1 }, "weighted_ca": 0.0, "weighted_ce": 1.0, "weighted_internal_edges": 0.0, "weighted_outgoing_edges": 1.0, "weighted_outgoing_to": { "core": 1.0 } },
            { "id": "cli.py", "files": 1, "ca": 0, "ce": 1, "instability": null,
              "internal_edges": 0, "outgoing_edges": 2, "cohesion": null,
              "outgoing_to": { "api": 2 }, "weighted_ca": 0.0, "weighted_ce": 0.0, "weighted_internal_edges": 0.0, "weighted_outgoing_edges": 0.0, "weighted_outgoing_to": { "api": 0.0 } },
            { "id": "core", "files": 2, "ca": 2, "ce": 1, "instability": 0.3333,
              "internal_edges": 0, "outgoing_edges": 2, "cohesion": 0.0,
              "outgoing_to": { "db": 2 }, "weighted_ca": 2.0, "weighted_ce": 1.0, "weighted_internal_edges": 0.0, "weighted_outgoing_edges": 2.0, "weighted_outgoing_to": { "db": 2.0 } },
            { "id": "db", "files": 3, "ca": 1, "ce": 1, "instability": 0.5,
              "internal_edges": 1, "outgoing_edges": 1, "cohesion": 0.5,
              "outgoing_to": { "core": 1 }, "weighted_ca": 1.0, "weighted_ce": 1.0, "weighted_internal_edges": 1.0, "weighted_outgoing_edges": 1.0, "weighted_outgoing_to": { "core": 1.0 } }
        ])
    );
    assert_eq!(
        doc["module_edges"],
        json!([
            { "from": "api", "to": "core", "imports": 1, "forbidden": 0 },
            { "from": "cli.py", "to": "api", "imports": 1, "forbidden": 0 },
            { "from": "core", "to": "db", "imports": 2, "forbidden": 0 },
            { "from": "db", "to": "core", "imports": 1, "forbidden": 0 }
        ])
    );
    // core -> db carries 2 imports, db -> core 1: an even pair, with no lighter side. No file cycle: service -> repo -> core/__init__
    // never returns to service.
    assert_eq!(
        doc["cycles"],
        json!([
            { "level": "module", "members": ["core", "db"], "exact": true, "grouping": true,
              "break": [{ "from": "mypkg/db/repo.py", "to": "mypkg/core/__init__.py" }],
              "two_way": [{ "from": "core", "to": "db", "imports": 2, "reverse_imports": 1,
                            "even": true,
                            "edges": [{ "from": "mypkg/core/__init__.py", "to": "mypkg/db/session.py" },
                                      { "from": "mypkg/core/service.py", "to": "mypkg/db/repo.py" }] }] }
        ])
    );

    let findings = doc["findings"].as_array().unwrap();
    let kinds: Vec<(&str, &str)> = findings
        .iter()
        .map(|f| (f["kind"].as_str().unwrap(), f["subject"].as_str().unwrap()))
        .collect();
    // repo.py imports core/__init__.py, but only service.py of core imports it: not misplaced.
    // core/__init__.py cannot move without its package, and cli.py is alone in its module.
    assert_eq!(kinds, [("seam", ".")]);
    let cycle = common::finding(&doc, "cycle", "core, db");
    assert_eq!(cycle["evidence"], doc["cycles"][0]);
    assert!(
        cycle.get("recipe").is_none(),
        "folder/re-export coupling is not an actionable cut"
    );
    // repo.py has degree 3, above the 90th percentile (2), so it is a hub and left out.
    // Q(clusters) = 3/4 - (6/8)^2 + 1/4 - (2/8)^2; Q(folders) = -18/64. The clusters hold 6 of
    // the 9 non-barrel files, at least half.
    assert_eq!(
        findings[0]["evidence"],
        json!({ "modularity": 0.375, "folder_modularity": -0.2813,
                "hubs": ["mypkg/db/repo.py"], "via_barrel": true, "clusters": [
                    { "files": ["mypkg/api/handlers.py", "mypkg/api/schemas.py", "mypkg/cli.py",
                                "mypkg/core/service.py"], "modules": ["api", "cli.py", "core"] },
                    { "files": ["mypkg/core/__init__.py", "mypkg/db/session.py"],
                      "modules": ["core", "db"] } ] })
    );
    assert_eq!(
        common::finding(&doc, "small-repo", ".")["evidence"],
        json!({ "files": 10, "modules": 5 })
    );
}

#[test]
fn python_package_detection_finds_namespace_packages_and_skips_tests() {
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
    let dump = json!({ "grimp_version": "3.17", "modules": ["app", "app.service"], "imports": [] });
    let input = common::write_tmp("namespace-package.json", &dump.to_string());
    let args = [
        root.to_str().unwrap(),
        "--language",
        "python",
        "--collector-input",
        &input,
    ];
    let (doc, _, _) = common::scan(&args);
    assert_eq!(doc["config"]["source_root"], "app");

    // With tests kept, two packages: ambiguous.
    let out = common::archgraph(&[&["scan"], &args[..], &["--include-tests"]].concat());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && err.contains("app, tests"), "{err}");
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

#[test]
fn python_deferred_imports_are_type_or_dynamic_and_close_no_cycle() {
    // b/y.py imports a/x.py only under `if TYPE_CHECKING:` (type) and c/z.py only inside a method
    // (dynamic); c/z.py imports d/w.py only inside a method (dynamic); d/w.py imports c/z.py at
    // module level and inside a method: one load-time occurrence makes it static. Every would-be
    // cycle (a <-> b, b <-> c, c <-> d) closes only through a deferred import: no cycle.
    let (doc, _, _) = scan_recorded("py-deferred", &[]);
    let kinds: Vec<(&str, &str, &str)> = doc["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["from"].as_str().unwrap(),
                e["to"].as_str().unwrap(),
                e["kind"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("deferred/a/x.py", "deferred/b/y.py", "runtime"),
            ("deferred/b/y.py", "deferred/a/x.py", "type"),
            ("deferred/b/y.py", "deferred/c/z.py", "lazy"),
            ("deferred/c/z.py", "deferred/b/y.py", "runtime"),
            ("deferred/c/z.py", "deferred/d/w.py", "lazy"),
            ("deferred/d/w.py", "deferred/c/z.py", "runtime"),
        ]
    );
    assert_eq!(doc["cycles"], json!([]));
    // They still count in metrics: b imports a and c.
    let b = doc["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "b")
        .unwrap();
    assert_eq!(b["ce"], 2);
    assert_eq!(b["weighted_ce"], 0.4);
    assert_eq!(b["weighted_ca"], 2.0);
    assert_eq!(b["instability"], 0.1667);
    assert_eq!(doc["summary"]["type_only_cycles"], 1);
}
