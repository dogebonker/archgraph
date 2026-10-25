mod common;

use common::{archgraph, finding, layout, scan, scan_file, scan_recorded, write_tmp};
use serde_json::{Value, json};

fn edges(doc: &Value) -> Vec<(&str, &str)> {
    doc["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            assert_eq!(e["kind"], "runtime");
            (e["from"].as_str().unwrap(), e["to"].as_str().unwrap())
        })
        .collect()
}

fn files(doc: &Value) -> Vec<&str> {
    doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect()
}

#[test]
fn go_references_resolve_to_the_declaring_files() {
    // No --language: go.mod and .go files only.
    let (doc, _, stderr) = scan_recorded("go-resolve", &[]);
    assert_eq!(doc["source"]["language"], "go");
    assert_eq!(
        doc["source"]["collector"],
        json!({ "name": "go-types", "version": "1.27.1", "cruised": 23, "unresolved": 2,
                "main_packages": ["cmd/tool"],
                // Methods of a type in another file (`together`), or unexported names of another
                // file: none of these files could leave its package and still compile. A field
                // is named by its struct type, also as a composite-literal key.
                "bound": [
                    { "from": "server/handle.go", "to": "server/server.go", "names": ["Server.db"],
                      "together": true },
                    { "from": "server/server.go", "to": "server/base.go", "names": ["base"] },
                    { "from": "store/open.go", "to": "store/db.go", "names": ["DB.values"] },
                    { "from": "store/size.go", "to": "store/native.go", "names": ["native"] },
                    { "from": "store/size.go", "to": "store/sys_linux.go", "names": ["pageSize"] },
                    { "from": "store/size.go", "to": "store/sys_windows.go", "names": ["pageSize"] },
                    { "from": "store/sys_linux.go", "to": "store/sys_windows.go", "together": true }],
                "go_modules": ["."] })
    );
    assert!(
        stderr.contains("2 unresolved collector reference(s) reported by go-types"),
        "{stderr}"
    );
    assert_eq!(doc["config"]["source_root"], ".");
    // gen/gen.go needs the `ignore` tag: no file. native.go (cgo) and both platform files are.
    assert_eq!(
        files(&doc),
        [
            "blank/beta.go",
            "cmd/tool/main.go",
            "kinds/alias.go",
            "kinds/emb.go",
            "kinds/gen.go",
            "kinds/iface.go",
            "kinds/impl.go",
            "lost/lost.go",
            "plugin/plugin.go",
            "server/base.go",
            "server/handle.go",
            "server/server.go",
            "store/db.go",
            "store/doc.go",
            "store/native.go",
            "store/open.go",
            "store/size.go",
            "store/sys_linux.go",
            "store/sys_windows.go",
            "use/use.go",
            "wrap/wrap.go",
        ]
    );
    // Written by hand from the sources. `s.db.Get` reaches the field's file (server.go) and the
    // method's (db.go); `s.Name()` the promoted method's file (base.go); pageSize, declared for
    // two platforms, both files; the blank import of store its doc.go, of blank its one file
    // that is no test (beta.go, not alpha_test.go); the missing package nothing (unresolved), nor
    // lost.go's import of kinds, whose one use the type checker could not see (unresolved).
    // An interface method, also through an embedding interface, leads to the interface's file, a
    // generic type's method to its file, an alias to the alias's file.
    assert_eq!(
        edges(&doc),
        [
            ("cmd/tool/main.go", "server/base.go"),
            ("cmd/tool/main.go", "server/handle.go"),
            ("cmd/tool/main.go", "server/server.go"),
            ("kinds/alias.go", "kinds/impl.go"),
            ("kinds/emb.go", "kinds/iface.go"),
            ("lost/lost.go", "wrap/wrap.go"),
            ("plugin/plugin.go", "blank/beta.go"),
            ("plugin/plugin.go", "store/doc.go"),
            ("server/handle.go", "server/server.go"),
            ("server/handle.go", "store/db.go"),
            ("server/server.go", "server/base.go"),
            ("server/server.go", "store/db.go"),
            ("server/server.go", "store/open.go"),
            ("store/open.go", "store/db.go"),
            ("store/size.go", "store/native.go"),
            ("store/size.go", "store/sys_linux.go"),
            ("store/size.go", "store/sys_windows.go"),
            ("use/use.go", "kinds/alias.go"),
            ("use/use.go", "kinds/emb.go"),
            ("use/use.go", "kinds/gen.go"),
            ("use/use.go", "kinds/iface.go"),
        ]
    );
    // The external test package's use of use.Use.
    let (doc, _, _) = scan_recorded("go-resolve", &["--include-tests"]);
    assert!(edges(&doc).contains(&("use/use_test.go", "use/use.go")));
}

#[test]
fn go_edges_carry_the_line_of_their_first_use() {
    // A use within one package has no import statement: the edge's own line is the only one.
    // A blank import's edge has the import's line.
    let (doc, _, _) = scan_recorded("go-resolve", &[]);
    let line = |from: &str, to: &str| {
        doc["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["from"] == from && e["to"] == to)
            .map(|e| e["line"].clone())
    };
    assert_eq!(line("store/size.go", "store/sys_linux.go"), Some(json!(3)));
    assert_eq!(line("plugin/plugin.go", "store/doc.go"), Some(json!(7)));
}

#[test]
fn go_cycles_appear_only_between_merged_packages() {
    // handler.go and route.go use each other: one package, which Go compiles as a unit, so no
    // file cycle. At --depth 1 api/handler -> store and store/sql -> api/types merge into a
    // module cycle.
    let (doc, _, _) = scan_recorded("go-cycles", &["--depth", "1"]);
    assert!(edges(&doc).contains(&("api/handler/handler.go", "api/handler/route.go")));
    assert!(edges(&doc).contains(&("api/handler/route.go", "api/handler/handler.go")));
    let cycles = doc["cycles"].as_array().unwrap();
    assert_eq!(cycles.len(), 1, "{cycles:#?}");
    assert_eq!(cycles[0]["level"], "module");
    assert_eq!(cycles[0]["members"], json!(["api", "store"]));
    assert_eq!(cycles[0]["grouping"], true);
    assert_eq!(doc["summary"]["cycles"], 1);
    // No package cycle: the report points at the depth, not at moving files.
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-cycles-report");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let input = common::fixture("go-cycles").join("go.json");
    let out = archgraph(&[
        "report",
        root.to_str().unwrap(),
        "--language",
        "go",
        "--depth",
        "1",
        "--collector-input",
        input.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("  cycle of api, store comes from module grouping: packages are acyclic; the cycle exists only at folder depth 1; change --depth or the module grouping\n"),
        "{stdout}"
    );

    // At the default depth every package is its own module: no cycle at all.
    let (doc, _, _) = scan_recorded("go-cycles", &[]);
    assert_eq!(doc["cycles"], json!([]));
}

#[test]
fn go_misplaced_file_moves_to_its_only_users() {
    let (doc, _, _) = scan_recorded("go-misplaced", &[]);
    // Only invoice uses them, but none can move: percent.go holds a method of rate.go's type,
    // tax.go calls util.go's unexported round, and notify_*.go are one function per platform;
    // pub/money.go is public API (outside internal/).
    assert_eq!(
        doc["source"]["collector"]["bound"],
        json!([
            { "from": "internal/report/notify_linux.go", "to": "internal/report/notify_other.go",
              "together": true },
            { "from": "internal/report/notify_linux.go", "to": "internal/report/notify_windows.go",
              "together": true },
            { "from": "internal/report/notify_other.go", "to": "internal/report/notify_windows.go",
              "together": true },
            { "from": "internal/report/percent.go", "to": "internal/report/rate.go",
              "together": true },
            { "from": "internal/report/tax.go", "to": "internal/report/util.go",
              "names": ["round"] }
        ])
    );
    let misplaced: Vec<&Value> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "misplaced-file")
        .map(|f| &f["subject"])
        .collect();
    assert_eq!(misplaced, [&json!("internal/report/format.go")]);
    let f = finding(&doc, "misplaced-file", "internal/report/format.go");
    assert_eq!(f["evidence"]["target_module"], "invoice");
    assert_eq!(f["evidence"]["imports_from_target"], 3);
    assert_eq!(
        f["suggestion"],
        json!({ "move": { "file": "internal/report/format.go", "to": "invoice/format.go" } })
    );
}

#[test]
fn excluding_a_platform_variant_keeps_the_remaining_variants_bound() {
    for (excluded, from, to) in [
        ("**/*_linux.go", "notify_other.go", "notify_windows.go"),
        (
            "internal/report/notify_linux.go",
            "notify_other.go",
            "notify_windows.go",
        ),
        (
            "internal/report/notify_other.go",
            "notify_linux.go",
            "notify_windows.go",
        ),
        (
            "internal/report/notify_windows.go",
            "notify_linux.go",
            "notify_other.go",
        ),
    ] {
        let (doc, _, _) = scan_recorded("go-misplaced", &["--exclude", excluded]);
        let (from, to) = (
            format!("internal/report/{from}"),
            format!("internal/report/{to}"),
        );
        assert!(
            doc["source"]["collector"]["bound"]
                .as_array()
                .unwrap()
                .contains(&json!({"from": from, "to": to, "together": true})),
            "remaining variants lost their binding after excluding {excluded}"
        );
        assert!(
            doc["findings"].as_array().unwrap().iter().all(|finding| {
                finding["kind"] != "misplaced-file"
                    || (finding["subject"] != from && finding["subject"] != to)
            }),
            "a remaining variant was offered an individual move after excluding {excluded}"
        );
    }
}

#[test]
fn go_layers_violation_fails_check() {
    let graph = scan_file("go-layers", &["--rules", &layout("go-layers")]);
    let out = archgraph(&["check", &graph]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert_eq!(
        stdout,
        "forbidden domain/order.go -> adapters/postgres/repo.go (layers)\n1 forbidden, 0 unplanned\n"
    );
}

#[test]
fn go_internals_reach_comes_only_from_public_rules() {
    let (doc, _, _) = scan_recorded("go-internals", &["--rules", &layout("go-internals")]);
    let f = finding(&doc, "internals-reach", "billing/**");
    assert_eq!(
        f["evidence"]["edges"],
        json!([{ "from": "orders/checkout.go", "to": "billing/ledger/ledger.go" }])
    );
    // Without rules nothing: the compiler already guards internal/ folders.
    let (doc, _, _) = scan_recorded("go-internals", &[]);
    assert!(
        doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["kind"] != "internals-reach")
    );
}

#[test]
fn go_excludes_tests_generated_code_and_vendor_and_joins_replaced_modules() {
    // lib is a second module, joined by `replace`; vendor/ is never listed.
    let (doc, _, stderr) = scan_recorded("go-excludes", &[]);
    assert_eq!(
        doc["source"]["collector"]["go_modules"],
        json!([".", "lib"])
    );
    // Test support is left out file by file: testhelp.go uses testing.TB and nothing outside
    // package app uses it. finder.go uses testing too but app/cmd uses its Finder; flag.go only
    // calls testing.Testing; probe_windows.go uses testing.T but its variant probe_other.go does
    // not; in app/chain the variants of A and B use testing but c_darwin.go does not: all stay.
    // apptesting is test support by its name, check by its one file: the warning names packages
    // left out whole and single files.
    assert_eq!(
        files(&doc),
        [
            "app/app.go",
            "app/chain/a_linux.go",
            "app/chain/b_windows.go",
            "app/chain/c_darwin.go",
            "app/cmd/cmd.go",
            "app/finder.go",
            "app/flag.go",
            "app/probe_other.go",
            "app/probe_windows.go",
            "lib/lib.go"
        ]
    );
    assert!(
        stderr.contains(
            "warning: left out as test support (--include-tests keeps them): app/apptesting/, \
             app/check/, app/testhelp.go\n"
        ),
        "{stderr}"
    );
    assert_eq!(
        edges(&doc),
        [
            ("app/app.go", "lib/lib.go"),
            ("app/cmd/cmd.go", "app/finder.go"),
            ("app/flag.go", "app/probe_other.go"),
            ("app/flag.go", "app/probe_windows.go"),
        ]
    );
    // app.go is bound to app_test.go (a method of Runner) and wire_gen.go (unexported wired),
    // both excluded here: so it is not bound, and a limitation says so.
    let bound = [
        json!({ "from": "app/chain/a_linux.go", "to": "app/chain/b_windows.go", "together": true }),
        json!({ "from": "app/chain/b_windows.go", "to": "app/chain/c_darwin.go", "together": true }),
        json!({ "from": "app/flag.go", "to": "app/probe_other.go", "names": ["probe"] }),
        json!({ "from": "app/flag.go", "to": "app/probe_windows.go", "names": ["probe"] }),
        json!({ "from": "app/probe_other.go", "to": "app/probe_windows.go", "together": true }),
    ];
    assert_eq!(doc["source"]["collector"]["bound"], json!(bound));
    assert_eq!(
        doc["source"]["collector"]["limitations"],
        json!([
            "Go: 2 bound pairs with an excluded file (a test, generated code) are left out, so moving their other file may need more names exported"
        ])
    );
    let excludes = doc["config"]["excludes"].as_array().unwrap();
    for glob in ["**/*.pb.go", "**/*_test.go"] {
        assert!(excludes.contains(&json!(glob)), "{glob}");
    }

    let (doc, _, _) = scan_recorded("go-excludes", &["--include-tests"]);
    let test_pair = json!({ "from": "app/app_test.go", "to": "app/app.go", "together": true });
    assert_eq!(
        doc["source"]["collector"]["bound"],
        json!([std::slice::from_ref(&test_pair), &bound[..]].concat())
    );
    assert_eq!(
        files(&doc),
        [
            "app/app.go",
            "app/app_test.go",
            "app/apptesting/fake.go",
            "app/chain/a_linux.go",
            "app/chain/b_windows.go",
            "app/chain/c_darwin.go",
            "app/check/check.go",
            "app/cmd/cmd.go",
            "app/ext_test.go",
            "app/finder.go",
            "app/flag.go",
            "app/probe_other.go",
            "app/probe_windows.go",
            "app/testhelp.go",
            "lib/lib.go"
        ]
    );
    assert_eq!(
        edges(&doc),
        [
            ("app/app.go", "lib/lib.go"),
            ("app/app_test.go", "app/app.go"),
            ("app/apptesting/fake.go", "app/app.go"),
            ("app/cmd/cmd.go", "app/finder.go"),
            ("app/ext_test.go", "app/app.go"),
            ("app/flag.go", "app/probe_other.go"),
            ("app/flag.go", "app/probe_windows.go"),
            ("app/testhelp.go", "app/app.go"),
        ]
    );

    // Generated files (by their `Code generated ... DO NOT EDIT.` header) come back too.
    let (doc, _, _) = scan_recorded("go-excludes", &["--no-default-excludes"]);
    let wire_pair = json!({ "from": "app/app.go", "to": "app/wire_gen.go", "names": ["wired"] });
    assert_eq!(
        doc["source"]["collector"]["bound"],
        json!([&[wire_pair, test_pair], &bound[..]].concat())
    );
    assert_eq!(
        files(&doc),
        [
            "app/app.go",
            "app/app.pb.go",
            "app/app_test.go",
            "app/apptesting/fake.go",
            "app/chain/a_linux.go",
            "app/chain/b_windows.go",
            "app/chain/c_darwin.go",
            "app/check/check.go",
            "app/cmd/cmd.go",
            "app/ext_test.go",
            "app/finder.go",
            "app/flag.go",
            "app/probe_other.go",
            "app/probe_windows.go",
            "app/testhelp.go",
            "app/wire_gen.go",
            "lib/lib.go"
        ]
    );
    assert!(edges(&doc).contains(&("app/app.go", "app/wire_gen.go")));
}

#[test]
fn go_is_detected_only_beside_its_root_marker() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-detect");
    let _ = std::fs::remove_dir_all(&root);
    for file in ["a/a.go", "a/b.go", "web/main.ts"] {
        write_tmp(&format!("go-detect/{file}"), "");
    }
    let input = write_tmp(
        "go-detect.json",
        r#"{ "modules": [], "summary": { "totalCruised": 0 } }"#,
    );
    let detected = |root: &std::path::Path| {
        let out = archgraph(&["scan", root.to_str().unwrap(), "--collector-input", &input]);
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr)
    };
    // Without go.mod the .go files do not count: TypeScript, and the empty recording fails.
    assert!(
        detected(&root).contains("cruised 0 files"),
        "{}",
        detected(&root)
    );
    // With it, two .go files beat one .ts file.
    write_tmp("go-detect/go.mod", "module example.com/d\n\ngo 1.21\n");
    let go = write_tmp(
        "go-detect-go.json",
        r#"{ "go_version": "1.27.1", "files": [{ "path": "a/a.go", "generated": false }],
             "edges": [], "unresolved": 0 }"#,
    );
    let (doc, _, _) = scan(&[root.to_str().unwrap(), "--collector-input", &go]);
    assert_eq!(doc["source"]["language"], "go");
    // A tie keeps TypeScript.
    write_tmp("go-detect/web/other.ts", "");
    assert!(
        detected(&root).contains("cruised 0 files"),
        "{}",
        detected(&root)
    );
}

#[test]
fn go_package_cycle_without_a_file_cycle_is_no_grouping() {
    // a/use.go imports b, and b imports a/a.go: no file cycle, but packages a and b import
    // each other, which Go rejects. The cycle is real, not folder grouping.
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-package-cycle");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let dump = write_tmp(
        "go-package-cycle.json",
        r#"{ "go_version": "1.27.1", "unresolved": 0, "modules": ["."],
             "files": [{ "path": "a/a.go", "generated": false },
                       { "path": "a/use.go", "generated": false },
                       { "path": "b/b.go", "generated": false }],
             "edges": [{ "from": "a/use.go", "to": "b/b.go" }, { "from": "b/b.go", "to": "a/a.go" }] }"#,
    );
    let (doc, _, _) = scan(&[
        root.to_str().unwrap(),
        "--language",
        "go",
        "--collector-input",
        &dump,
    ]);
    assert_eq!(doc["cycles"][0]["members"], json!(["a", "b"]));
    assert_eq!(doc["cycles"][0]["grouping"], false, "{:#}", doc["cycles"]);
}

#[test]
fn go_files_in_the_root_are_one_package_and_one_module() {
    // A library whose package sits at the root (cobra): its files use each other in both
    // directions. As one module `.` they form no cycle; as a module per file they would.
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-root-package");
    let _ = std::fs::remove_dir_all(&root);
    write_tmp(
        "go-root-package/go.mod",
        "module example.com/lib\n\ngo 1.21\n",
    );
    let dump = write_tmp(
        "go-root-package.json",
        r#"{ "go_version": "1.27.1", "unresolved": 0,
             "files": [{ "path": "command.go", "generated": false },
                       { "path": "args.go", "generated": false },
                       { "path": "doc/gen.go", "generated": false }],
             "edges": [{ "from": "command.go", "to": "args.go" }, { "from": "args.go", "to": "command.go" },
                       { "from": "doc/gen.go", "to": "command.go" }] }"#,
    );
    let (doc, _, _) = scan(&[
        root.to_str().unwrap(),
        "--language",
        "go",
        "--collector-input",
        &dump,
    ]);
    let modules: Vec<(&str, &str)> = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap(), f["module"].as_str().unwrap()))
        .collect();
    assert_eq!(
        modules,
        [("args.go", "."), ("command.go", "."), ("doc/gen.go", "doc")]
    );
    assert_eq!(doc["cycles"], json!([]));
    // check, render and simulate recompute the same modules.
    let graph = write_tmp(
        "go-root-package-scan.json",
        &serde_json::to_string(&doc).unwrap(),
    );
    let out = archgraph(&["simulate", &graph, "--move", "doc/gen.go", "gen.go"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let sim: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(sim["move"]["module"], ".");
}

#[test]
fn go_moves_never_leave_their_go_module() {
    // go-misplaced's shape with internal/report a module of its own: the only candidate move,
    // internal/report/format.go into invoice, would cross a go.mod boundary.
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-two-modules");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let dump = |modules: &str| {
        let text = format!(
            r#"{{ "go_version": "1.27.1", "unresolved": 0, "modules": {modules},
                 "files": [{{ "path": "internal/report/format.go", "generated": false }},
                           {{ "path": "internal/report/summary.go", "generated": false }},
                           {{ "path": "invoice/create.go", "generated": false }},
                           {{ "path": "invoice/send.go", "generated": false }},
                           {{ "path": "invoice/total.go", "generated": false }},
                           {{ "path": "dashboard/view.go", "generated": false }}],
                 "edges": [{{ "from": "invoice/create.go", "to": "internal/report/format.go" }},
                           {{ "from": "invoice/send.go", "to": "internal/report/format.go" }},
                           {{ "from": "invoice/total.go", "to": "internal/report/format.go" }},
                           {{ "from": "dashboard/view.go", "to": "internal/report/summary.go" }}] }}"#
        );
        let input = write_tmp(&format!("go-two-modules-{}.json", modules.len()), &text);
        let (doc, _, _) = scan(&[
            root.to_str().unwrap(),
            "--language",
            "go",
            "--collector-input",
            &input,
        ]);
        doc
    };
    let misplaced = |doc: &Value| {
        doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "misplaced-file")
    };
    // One module: the move is proposed.
    assert!(misplaced(&dump(r#"["."]"#)));
    let doc = dump(r#"[".", "internal/report"]"#);
    assert_eq!(
        doc["source"]["collector"]["go_modules"],
        json!([".", "internal/report"])
    );
    assert!(!misplaced(&doc), "{:#}", doc["findings"]);
}

#[test]
fn go_modules_group_packages_by_depth() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-packages");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let dump = write_tmp(
        "go-packages.json",
        r#"{ "go_version": "1.27.1", "unresolved": 0, "modules": ["."],
             "files": [{ "path": "a/b/c/x.go", "generated": false },
                       { "path": "a/b/c/d/y.go", "generated": false },
                       { "path": "main.go", "generated": false }],
             "edges": [{ "from": "a/b/c/d/y.go", "to": "a/b/c/x.go" }] }"#,
    );
    let modules = |extra: &[&str]| {
        let args = [
            &[
                root.to_str().unwrap(),
                "--language",
                "go",
                "--collector-input",
                &dump,
            ][..],
            extra,
        ]
        .concat();
        let (doc, stdout, _) = scan(&args);
        // render recomputes the modules from the config: it must agree.
        let graph = write_tmp("go-packages-scan.json", &stdout);
        assert!(
            archgraph(&["render", &graph, "-o", "/dev/null"])
                .status
                .success()
        );
        let modules: Vec<String> = doc["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["module"].as_str().unwrap().to_owned())
            .collect();
        (modules, doc["config"].get("package_modules").cloned())
    };
    // The default depth 2 merges packages by folder as for other languages; the root's files
    // are `.`.
    assert_eq!(
        modules(&[]),
        (
            vec!["a/b".to_owned(), "a/b".to_owned(), ".".to_owned()],
            None
        )
    );
    // A large --depth makes each package (directory) a module.
    assert_eq!(
        modules(&["--depth", "99"]),
        (
            vec!["a/b/c/d".to_owned(), "a/b/c".to_owned(), ".".to_owned()],
            None
        )
    );
}

#[test]
fn go_module_discovery_skips_broken_and_excluded_go_mods() {
    use boundrail::collect::{globset, go};
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-discovery");
    let _ = std::fs::remove_dir_all(&root);
    for (file, text) in [
        ("go.mod", "module example.com/x\n\ngo 1.21\n"),
        ("lib/go.mod", "// a comment\nmodule \"example.com/x/lib\"\n"),
        ("empty/go.mod", ""),
        ("gen/go.mod", "go 1.21\n"),
        ("tools/go.mod", "module example.com/tools\n"),
        ("vendor/v/go.mod", "module example.com/v\n"),
        ("testdata/t/go.mod", "module example.com/t\n"),
        (".hidden/go.mod", "module example.com/h\n"),
    ] {
        write_tmp(&format!("go-discovery/{file}"), text);
    }
    let excludes = globset(&["tools/**".to_owned()]).unwrap();
    let (dirs, warnings) = go::module_dirs(&root, &excludes).unwrap();
    assert_eq!(dirs, [".", "lib"]);
    assert_eq!(
        warnings,
        [
            "skipping empty/go.mod: it has no module line",
            "skipping gen/go.mod: it has no module line"
        ]
    );
}

#[test]
fn go_scans_report_no_seam() {
    // Package a's first three files work with b, its last three with c: file clusters split a,
    // which Go cannot do, and package clusters compare with a package partition of modularity
    // near 0, so any grouping wins. Go reports no seam.
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-seam");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut files = Vec::new();
    let mut edges = Vec::new();
    for p in ["a", "b", "c", "d", "e"] {
        for i in 1..=6 {
            files.push(format!(
                r#"{{ "path": "{p}/f{i}.go", "generated": false }}"#
            ));
        }
    }
    for i in 1..=3 {
        for j in 1..=6 {
            edges.push(format!(r#"{{ "from": "a/f{i}.go", "to": "b/f{j}.go" }}"#));
            edges.push(format!(
                r#"{{ "from": "a/f{}.go", "to": "c/f{j}.go" }}"#,
                i + 3
            ));
        }
    }
    for j in 1..=5 {
        edges.push(format!(r#"{{ "from": "d/f{j}.go", "to": "e/f{j}.go" }}"#));
        edges.push(format!(
            r#"{{ "from": "b/f{j}.go", "to": "b/f{}.go" }}"#,
            j + 1
        ));
        edges.push(format!(
            r#"{{ "from": "c/f{j}.go", "to": "c/f{}.go" }}"#,
            j + 1
        ));
    }
    let dump = write_tmp(
        "go-seam.json",
        &format!(
            r#"{{ "go_version": "1.27.1", "unresolved": 0, "modules": ["."], "files": [{}], "edges": [{}] }}"#,
            files.join(","),
            edges.join(",")
        ),
    );
    let (doc, _, _) = scan(&[
        root.to_str().unwrap(),
        "--language",
        "go",
        "--collector-input",
        &dump,
    ]);
    assert!(
        doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["kind"] != "seam"),
        "{:#}",
        doc["findings"]
    );
}

#[test]
fn go_import_lines_count_new_importer_and_package_pairs() {
    // server.go leaves server/: main.go and handle.go gain an import of core/, server.go gains one
    // of server/ (base.go), and its import of store/ stays as it was.
    let rules = write_tmp(
        "rules/go-moves.yaml",
        "version: 1\nmoves:\n  - { from: \"server/server.go\", to: \"core/\" }\n",
    );
    let (doc, _, _) = scan_recorded("go-resolve", &["--rules", &rules]);
    assert_eq!(
        doc["target"]["moves"],
        json!([{ "slice": "*", "files": [{ "file": "server/server.go", "to": "core/server.go" }],
                 "import_lines": 3,
                 "names": [{ "name": "Server.db", "declared_in": "server/server.go",
                             "used_by": ["server/handle.go"] },
                           { "name": "base", "declared_in": "server/base.go",
                             "used_by": ["server/server.go"] }],
                 "together": [{ "from": "server/handle.go", "to": "server/server.go" }],
                 // handle.go imports server.go, which imports base.go back in server/.
                 "new_cycles": [{ "from": "core/server.go", "to": "server/base.go" },
                                { "from": "server/handle.go", "to": "core/server.go" }] }])
    );
    let saved = write_tmp("go-moves.json", &doc.to_string());
    let stdout = String::from_utf8(archgraph(&["check", &saved]).stdout).unwrap();
    assert!(
        stdout.contains(
            "move *: 1 file, 3 import lines, 2 names to export, 2 imports in new package cycles\n"
        ),
        "{stdout}"
    );
}

#[test]
fn go_moves_creating_no_cycle_or_landing_on_a_file_still_report_their_cost() {
    // open.go only imports its own package, so core/ never imports back.
    let rules = write_tmp(
        "rules/go-moves-open.yaml",
        "version: 1\nmoves:\n  - { from: \"store/open.go\", to: \"core/\" }\n",
    );
    let (doc, _, _) = scan_recorded("go-resolve", &["--rules", &rules]);
    assert_eq!(
        doc["target"]["moves"],
        json!([{ "slice": "*", "files": [{ "file": "store/open.go", "to": "core/open.go" }],
                 "import_lines": 2,
                 "names": [{ "name": "DB.values", "declared_in": "store/db.go",
                             "used_by": ["store/open.go"] }] }])
    );
    assert_eq!(doc["target"].get("notes"), None);
    // b/x.go already exists, so the moves cannot be simulated.
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-move-onto");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let dump = write_tmp(
        "go-move-onto.json",
        r#"{ "go_version": "1.27.1", "unresolved": 0, "modules": ["."],
             "files": [{ "path": "a/x.go", "generated": false },
                       { "path": "b/x.go", "generated": false },
                       { "path": "c/c.go", "generated": false }],
             "edges": [{ "from": "a/x.go", "to": "c/c.go" }] }"#,
    );
    let rules = write_tmp(
        "rules/go-moves-onto.yaml",
        "version: 1\nmoves:\n  - { from: \"a/x.go\", to: \"b/\" }\n",
    );
    let (doc, _, _) = scan(&[
        root.to_str().unwrap(),
        "--language",
        "go",
        "--collector-input",
        &dump,
        "--rules",
        &rules,
    ]);
    assert_eq!(
        doc["target"]["moves"],
        json!([{ "slice": "*", "files": [{ "file": "a/x.go", "to": "b/x.go" }],
                 "import_lines": 0, "names": [] }])
    );
    assert_eq!(
        doc["target"]["notes"],
        json!(["moves not simulated: b/x.go already exists"])
    );
}

#[test]
fn go_moves_list_the_unexported_names_to_export() {
    let moved = |name: &str, moves: &str| {
        let rules = write_tmp(
            &format!("rules/go-names-{name}.yaml"),
            &format!("version: 1\nmoves:\n{moves}"),
        );
        scan_recorded("go-resolve", &["--rules", &rules]).0["target"]["moves"][0].clone()
    };
    // handle.go selects the field db of Server: one qualified name, never a bare `db` beside it,
    // and it holds a method of Server, so it moves with server.go.
    let handle = moved(
        "handle",
        "  - { from: \"server/handle.go\", to: \"api/\" }\n",
    );
    assert_eq!(
        handle["names"],
        json!([{ "name": "Server.db", "declared_in": "server/server.go",
                 "used_by": ["server/handle.go"] }])
    );
    assert_eq!(
        handle["together"],
        json!([{ "from": "server/handle.go", "to": "server/server.go" }])
    );
    // open.go fills DB's unexported field values: moved alone it needs it exported, moved with
    // db.go into one folder it needs nothing.
    let open = moved("open", "  - { from: \"store/open.go\", to: \"core/\" }\n");
    assert_eq!(
        open["names"],
        json!([{ "name": "DB.values", "declared_in": "store/db.go", "used_by": ["store/open.go"] }])
    );
    let both = moved(
        "both",
        "  - { from: \"store/{open,db}.go\", to: \"core/\" }\n",
    );
    assert_eq!(both["files"].as_array().unwrap().len(), 2);
    assert_eq!(both["names"], json!([]));
    assert_eq!(both.get("together"), None);
}

#[test]
fn saved_bound_pairs_must_name_scanned_files_and_plain_names() {
    let (doc, ..) = scan_recorded("go-resolve", &[]);
    let rejects = |edit: &dyn Fn(&mut serde_json::Value), message: &str| {
        let mut saved = doc.clone();
        edit(&mut saved["source"]["collector"]["bound"][0]);
        let error = boundrail::validate_scan(&serde_json::from_value(saved).unwrap()).unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
    };
    rejects(
        &|pair| pair["to"] = json!("nowhere.go"),
        "names a file outside the scan",
    );
    rejects(
        &|pair| pair["names"] = json!(["Server.db\nforged"]),
        "control characters",
    );
}

#[test]
fn a_go_types_scan_saved_before_bound_pairs_leaves_names_unknown() {
    // Saved before pairs carried names: the collector lists bound files only.
    let (mut doc, ..) = scan_recorded("go-resolve", &[]);
    let collector = &mut doc["source"]["collector"];
    collector["bound_files"] = json!(["store/db.go", "store/open.go"]);
    collector.as_object_mut().unwrap().remove("bound");
    let saved = write_tmp("go-legacy-bound/scan.json", &doc.to_string());
    let rules = write_tmp(
        "go-legacy-bound/layout.yaml",
        "version: 1\nmoves:\n  - { from: \"store/open.go\", to: \"core/\" }\n",
    );
    let output = format!(
        "{}/go-legacy-bound/checked.json",
        env!("CARGO_TARGET_TMPDIR")
    );
    let out = archgraph(&["check", &saved, "--rules", &rules, "-o", &output]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let checked: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    assert_eq!(checked["target"]["moves"][0]["names"], json!(null));
    assert_eq!(
        checked["target"]["notes"],
        json!(["names to export unknown: the scan predates bound pairs, rescan to list them"])
    );
}

#[test]
fn report_summary_prints_the_target_block() {
    // The push path of the action shows `report`'s summary: the target block must be in it.
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-target-report");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let rules = write_tmp(
        "go-target-report.yaml",
        "version: 1\nmoves:\n  - { from: \"<name>/**\", to: \"core/<name>/\" }\n\
         forbid:\n  - { from: \"api/**\", to: \"store/**\" }\n",
    );
    // Pushes use report's exit status: a listed forbidden import passes.
    write_tmp(
        "go-target-report.known.yaml",
        "version: 1\nknown:\n  - { rule: \"forbid[0]\", from: \"api/handler/handler.go\", to: \"store/store.go\" }\n",
    );
    let input = common::fixture("go-cycles").join("go.json");
    let out = archgraph(&[
        "report",
        root.to_str().unwrap(),
        "--language",
        "go",
        "--rules",
        &rules,
        "--collector-input",
        input.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("Rules\n  1 forbidden (1 known): forbid[0] 1\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "Target\nmove api: 3 files, 1 import line\nmove store: 2 files, 1 import line\n\
             target: 0 of 0 slices conform, 0 of 2 slices moved\n"
        ),
        "{stdout}"
    );
}
