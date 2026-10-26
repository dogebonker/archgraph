mod common;

use common::{
    boundrail, finding, fixture, layout, scan, scan_file, scan_recorded, stable, synthetic,
    validate, validate_sarif, verdicts, write_tmp,
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn v(rows: &[(&str, &str, &str, &str)]) -> Vec<(String, String, String, String)> {
    rows.iter()
        .map(|r| (r.0.into(), r.1.into(), r.2.into(), r.3.into()))
        .collect()
}

fn check(args: &[&str]) -> (String, String, Option<i32>) {
    let out = boundrail(&[&["check"], args].concat());
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
        out.status.code(),
    )
}

#[test]
fn ts_fda_verdicts_follow_the_seven_steps() {
    let rules = layout("ts-fda");
    let (doc, _, _) = scan_recorded("ts-fda", &["--rules", &rules]);

    let embedded = &doc["config"]["rules"];
    assert_eq!(embedded["path"], "layout.yaml");
    let hash: String = Sha256::digest(std::fs::read(&rules).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(embedded["sha256"], hash);
    assert_eq!(embedded["doc"]["version"], 1);
    assert_eq!(embedded["doc"]["forbid"][1]["id"], "browser-no-server");
    assert_eq!(embedded["doc"]["forbid"][1]["reachable"], true);

    assert_eq!(
        verdicts(&doc),
        v(&[
            (
                "src/domains/cart/client.ts",
                "src/domains/cart/types.ts",
                "allowed",
                "layers"
            ),
            (
                "src/domains/cart/controller.ts",
                "src/domains/cart/client.ts",
                "allowed",
                "layers"
            ),
            // clock.ts is in no layer, no module pattern and no rule.
            (
                "src/domains/cart/controller.ts",
                "src/lib/clock.ts",
                "unplanned",
                "none"
            ),
            (
                "src/domains/cart/ui/Cart.tsx",
                "src/domains/cart/controller.ts",
                "allowed",
                "layers"
            ),
            // Cross-domain internal: step 3.
            (
                "src/domains/cart/ui/Cart.tsx",
                "src/domains/checkout/client.ts",
                "forbidden",
                "public"
            ),
            // Trace 1: steps 1-2 no match, 3 skips the entry, 4 index is in no layer, 5 other
            // module, 6 allows the public entry.
            (
                "src/domains/cart/ui/Cart.tsx",
                "src/domains/checkout/types.ts",
                "allowed",
                "public"
            ),
            // Trace 2: layers alone would allow client -> model.server.
            (
                "src/domains/checkout/client.ts",
                "src/domains/checkout/model.server.ts",
                "forbidden",
                "browser-no-server"
            ),
            // Reachable: client.ts is not server code but reaches model.server.ts.
            (
                "src/domains/checkout/controller.ts",
                "src/domains/checkout/client.ts",
                "forbidden",
                "browser-no-server"
            ),
            // Trace 3: the server re-export through the browser index.
            (
                "src/domains/checkout/index.ts",
                "src/domains/checkout/model.server.ts",
                "forbidden",
                "browser-no-server"
            ),
            (
                "src/domains/checkout/index.ts",
                "src/domains/checkout/types.ts",
                "allowed",
                "same-module"
            ),
            (
                "src/domains/checkout/model.server.ts",
                "src/domains/checkout/types.ts",
                "allowed",
                "layers"
            ),
            (
                "src/domains/checkout/server.ts",
                "src/domains/checkout/model.server.ts",
                "allowed",
                "same-module"
            ),
            (
                "src/shared/format.ts",
                "src/domains/cart/types.ts",
                "forbidden",
                "shared-no-domain"
            ),
        ])
    );
    assert_eq!(
        doc["module_edges"],
        json!([
            { "from": "domains/cart", "to": "domains/checkout", "imports": 2, "forbidden": 1 },
            { "from": "domains/cart", "to": "lib", "imports": 1, "forbidden": 0 },
            { "from": "shared", "to": "domains/cart", "imports": 1, "forbidden": 1 }
        ])
    );
    assert_eq!(
        finding(&doc, "internals-reach", "src/domains/checkout/**")["evidence"],
        json!({ "module": "domains/checkout",
                "entries": ["src/domains/checkout/index.ts", "src/domains/checkout/server.ts"],
                "imports": 1, "importers": 1, "importing_files": 1,
                "edges": [{ "from": "src/domains/cart/ui/Cart.tsx",
                            "to": "src/domains/checkout/client.ts" }] })
    );
}

#[test]
fn ts_fda_check_prints_violations_and_exits_one() {
    let plain = scan_file("ts-fda", &[]);
    let (stdout, _, code) = check(&[&plain, "--rules", &layout("ts-fda")]);
    assert_eq!(code, Some(1));
    assert_eq!(
        stdout,
        "unplanned src/domains/cart/controller.ts -> src/lib/clock.ts (none)\n\
         forbidden src/domains/cart/ui/Cart.tsx -> src/domains/checkout/client.ts (public)\n\
         forbidden src/domains/checkout/client.ts -> src/domains/checkout/model.server.ts (browser-no-server) reason=\"browser graph never reaches server code\"\n\
         forbidden src/domains/checkout/controller.ts -> src/domains/checkout/client.ts (browser-no-server) \
         via src/domains/checkout/client.ts -> src/domains/checkout/model.server.ts reason=\"browser graph never reaches server code\"\n\
         forbidden src/domains/checkout/index.ts -> src/domains/checkout/model.server.ts (browser-no-server) reason=\"browser graph never reaches server code\"\n\
         forbidden src/shared/format.ts -> src/domains/cart/types.ts (shared-no-domain) reason=\"shared imports no domain\"\n\
         5 forbidden, 1 unplanned\n\
         claim shared-no-domain: broken reason=\"shared imports no domain\"\n\
         \x20 src/shared/format.ts -> src/domains/cart/types.ts\n\
         claim browser-no-server: broken reason=\"browser graph never reaches server code\"\n\
         \x20 src/domains/checkout/client.ts -> src/domains/checkout/model.server.ts\n\
         \x20 src/domains/checkout/controller.ts -> src/domains/checkout/client.ts\n\
         \x20 src/domains/checkout/index.ts -> src/domains/checkout/model.server.ts\n\
         target: 0 of 0 slices conform\n"
    );

    // check recomputes through the scan pipeline: a rule-less scan checked with rules equals a
    // scan with those rules.
    let out = write_tmp("ts-fda-checked.json", "");
    let (_, _, code) = check(&[&plain, "--rules", &layout("ts-fda"), "-o", &out]);
    assert_eq!(code, Some(1));
    let checked = std::fs::read_to_string(&out).unwrap();
    validate(&serde_json::from_str(&checked).unwrap());
    let (_, with_rules, _) = scan_recorded("ts-fda", &["--rules", &layout("ts-fda")]);
    assert_eq!(stable(&checked), stable(&with_rules));

    // SARIF: each broken claim import, then the forbidden import under no claim, at its line,
    // under the fixture's directory, which is the Git scan root.
    let (stdout, _, code) = check(&[&plain, "--rules", &layout("ts-fda"), "--format", "sarif"]);
    assert_eq!(code, Some(1));
    let sarif: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    validate_sarif(&sarif);
    let results = sarif["runs"][0]["results"].as_array().unwrap();
    let rows: Vec<(&str, &str, u64)> = results
        .iter()
        .map(|r| {
            let location = &r["locations"][0]["physicalLocation"];
            (
                r["ruleId"].as_str().unwrap(),
                location["artifactLocation"]["uri"].as_str().unwrap(),
                location["region"]["startLine"].as_u64().unwrap(),
            )
        })
        .collect();
    let root = "tests/fixtures/ts-fda/src/";
    assert_eq!(
        rows,
        [
            ("shared-no-domain", &*format!("{root}shared/format.ts"), 1),
            (
                "browser-no-server",
                &format!("{root}domains/checkout/client.ts"),
                1
            ),
            (
                "browser-no-server",
                &format!("{root}domains/checkout/controller.ts"),
                1
            ),
            (
                "browser-no-server",
                &format!("{root}domains/checkout/index.ts"),
                1
            ),
            ("public", &format!("{root}domains/cart/ui/Cart.tsx"), 3),
        ]
    );
}

/// Slice a imports its domain, b has both sides and holds, c has only a domain.
fn sliced_scan(name: &str, rules: &str) -> serde_json::Value {
    edges_scan(
        name,
        &[
            ("src/m/a/app/x.ts", "src/m/a/domain/d.ts"),
            ("src/m/b/app/y.ts", "src/shared/id.ts"),
            ("src/m/b/domain/e.ts", "src/shared/id.ts"),
            ("src/m/c/domain/f.ts", "src/shared/id.ts"),
        ],
        rules,
    )
}

/// A TypeScript scan of `edges` between files that need not exist, with `rules`.
fn edges_scan(name: &str, edges: &[(&str, &str)], rules: &str) -> serde_json::Value {
    let edges: Vec<(String, String)> = edges
        .iter()
        .map(|(f, t)| (f.to_string(), t.to_string()))
        .collect();
    let mut files: Vec<&String> = edges.iter().flat_map(|(f, t)| [f, t]).collect();
    files.sort();
    files.dedup();
    let (dir, input) = synthetic(name, files.len(), &edges);
    let rules = write_tmp(&format!("rules/{name}.yaml"), rules);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--rules",
        &rules,
    ]);
    doc
}

#[test]
fn a_claim_reports_a_verdict_over_its_bound_slices() {
    let doc = sliced_scan(
        "target-claims",
        "version: 1\nforbid:\n\
         \x20 - { id: app-no-domain, from: \"src/m/<name>/app/**\", to: \"src/m/<name>/domain/**\", reason: \"apps never reach a domain\" }\n\
         \x20 - { id: nowhere, from: \"src/nowhere/**\", to: \"src/nowhere/**\", reason: \"nothing\" }\n\
         \x20 - { id: app-no-infra, from: \"src/m/<name>/app/**\", to: \"src/m/<name>/infra/**\", reason: \"no infra\" }\n\
         \x20 - { id: unclaimed, from: \"src/m/*/app/**\", to: \"src/m/*/domain/**\" }\n",
    );
    assert_eq!(
        doc["target"],
        json!({
            "claims": [
                { "id": "app-no-domain", "reason": "apps never reach a domain", "source": null,
                  "verdict": "holds_in",
                  "slices": [
                      { "slice": "a", "files": ["src/m/a/app/x.ts", "src/m/a/domain/d.ts"],
                        "broken": [{ "from": "src/m/a/app/x.ts", "to": "src/m/a/domain/d.ts" }] },
                      { "slice": "b", "files": ["src/m/b/app/y.ts", "src/m/b/domain/e.ts"],
                        "broken": [] }
                  ],
                  "notes": [] },
                { "id": "nowhere", "reason": "nothing", "source": null, "verdict": "vacuous",
                  "slices": [], "notes": ["forbid[1] `from` matches no file"] },
                { "id": "app-no-infra", "reason": "no infra", "source": null, "verdict": "vacuous",
                  "slices": [], "notes": ["forbid[2] `to` matches no file"] }
            ],
            "progress": { "conforming": { "done": 1, "of": 2 } }
        })
    );
}

#[test]
fn an_earlier_rule_catching_the_import_leaves_the_claim_broken() {
    let doc = sliced_scan(
        "target-first-match",
        "version: 1\nforbid:\n\
         \x20 - { from: \"src/m/*/app/**\", to: \"src/m/*/domain/**\" }\n\
         \x20 - { id: app-no-domain, from: \"src/m/a/app/**\", to: \"src/m/a/domain/**\", reason: \"apps never reach a domain\" }\n",
    );
    let edge = verdicts(&doc)
        .into_iter()
        .find(|r| r.0 == "src/m/a/app/x.ts")
        .unwrap();
    assert_eq!(
        (edge.2.as_str(), edge.3.as_str()),
        ("forbidden", "forbid[0]")
    );
    assert_eq!(doc["target"]["claims"][0]["verdict"], "broken");
    assert_eq!(
        doc["target"]["claims"][0]["slices"][0]["broken"],
        json!([{ "from": "src/m/a/app/x.ts", "to": "src/m/a/domain/d.ts" }])
    );
}

#[test]
fn moves_report_pending_files_import_lines_and_slices_moved() {
    let rules = "version: 1\nmoves:\n\
         \x20 - { from: \"src/m/<name>/app/**\", to: \"src/app/<name>/\" }\n\
         \x20 - { from: \"src/old/<name>/**\", to: \"src/new/<name>/\" }\n";
    let doc = edges_scan(
        "target-moves",
        &[
            // Slice a has not moved: a moved importer, a moved target, and both.
            ("src/m/a/app/x.ts", "src/shared/id.ts"),
            ("src/shared/util.ts", "src/m/a/app/sub/y.ts"),
            ("src/m/a/app/x.ts", "src/m/a/app/sub/y.ts"),
            // Slice b is already at its target.
            ("src/app/b/z.ts", "src/shared/id.ts"),
        ],
        rules,
    );
    assert_eq!(
        doc["target"],
        json!({
            "claims": [],
            "moves": [
                { "slice": "a",
                  "files": [
                      { "file": "src/m/a/app/sub/y.ts", "to": "src/app/a/sub/y.ts" },
                      { "file": "src/m/a/app/x.ts", "to": "src/app/a/x.ts" }
                  ],
                  "import_lines": 3, "names": null },
                { "slice": "b", "files": [], "import_lines": 0, "names": null }
            ],
            "progress": { "conforming": { "done": 0, "of": 0 }, "moved": { "done": 1, "of": 2 } },
            "notes": ["moves[1] matches no file"]
        })
    );
    let saved = write_tmp("target-moves.json", &doc.to_string());
    let (stdout, _, code) = check(&[&saved]);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "unplanned src/app/b/z.ts -> src/shared/id.ts (none)\n\
         unplanned src/m/a/app/x.ts -> src/shared/id.ts (none)\n\
         unplanned src/shared/util.ts -> src/m/a/app/sub/y.ts (none)\n\
         0 forbidden, 3 unplanned\n\
         move a: 2 files, 3 import lines\n\
         note: moves[1] matches no file\n\
         target: 0 of 0 slices conform, 1 of 2 slices moved\n"
    );
}

#[test]
fn exported_exact_moves_check_as_one_slice_with_the_destinations_the_preview_showed() {
    // The what-if "Copy layout moves" output for app/[id].ts and app/helper.ts moved to web/.
    let rules = "version: 1\nmoves:   # exact paths, first match wins: keep these above other moves\n\
         \x20 - { from: \"app/[[]id[]].ts\", to: \"web/\" }\n\
         \x20 - { from: \"app/helper.ts\", to: \"web/\" }\n";
    let doc = edges_scan(
        "target-exported-moves",
        &[
            ("app/[id].ts", "app/helper.ts"),
            ("lib/a.ts", "app/helper.ts"),
        ],
        rules,
    );
    assert_eq!(
        doc["target"]["moves"],
        json!([{ "slice": "*",
                 "files": [
                     { "file": "app/[id].ts", "to": "web/[id].ts" },
                     { "file": "app/helper.ts", "to": "web/helper.ts" }
                 ],
                 "import_lines": 2, "names": null }])
    );
    assert_eq!(
        doc["target"]["progress"]["moved"],
        json!({ "done": 0, "of": 0 })
    );
}

#[test]
fn a_brace_claim_flags_a_moved_handler_importing_an_unmoved_repository() {
    let both = |role: &str| format!("{{src/m/<name>/{role}*.ts,src/app/<name>/{role}/*.ts}}");
    let rules = format!(
        "version: 1\nforbid:\n\
         \x20 - {{ id: layered, from: \"{}\", to: \"{}\", reason: \"inward\" }}\n\
         moves:\n\
         \x20 - {{ from: \"src/m/<name>/handler*.ts\", to: \"src/app/<name>/handler/\" }}\n\
         \x20 - {{ from: \"src/m/<name>/repository*.ts\", to: \"src/app/<name>/repository/\" }}\n",
        both("handler"),
        both("repository"),
    );
    let doc = edges_scan(
        "target-mid-migration",
        &[
            // u has not moved, v has, w moved its handler only.
            ("src/m/u/handler.ts", "src/shared/id.ts"),
            ("src/m/u/repository.ts", "src/shared/id.ts"),
            ("src/app/v/handler/handler.ts", "src/shared/id.ts"),
            ("src/app/v/repository/repository.ts", "src/shared/id.ts"),
            ("src/app/w/handler/handler.ts", "src/m/w/repository.ts"),
        ],
        &rules,
    );
    assert_eq!(
        doc["target"]["claims"][0],
        json!({ "id": "layered", "reason": "inward", "source": null, "verdict": "holds_in",
                "slices": [
                    { "slice": "u", "files": ["src/m/u/handler.ts", "src/m/u/repository.ts"],
                      "broken": [] },
                    { "slice": "v",
                      "files": ["src/app/v/handler/handler.ts", "src/app/v/repository/repository.ts"],
                      "broken": [] },
                    { "slice": "w", "files": ["src/app/w/handler/handler.ts", "src/m/w/repository.ts"],
                      "broken": [{ "from": "src/app/w/handler/handler.ts", "to": "src/m/w/repository.ts" }] }
                ],
                "notes": [] })
    );
    assert_eq!(
        doc["target"]["progress"],
        json!({ "conforming": { "done": 2, "of": 3 }, "moved": { "done": 1, "of": 3 } })
    );
}

#[test]
fn moves_list_package_cycles_for_go_only() {
    // A file cycle that moves as a whole is the same cycle at new paths, not a new one.
    let doc = edges_scan(
        "target-ts-cycles",
        &[
            ("src/m/a/app/x.ts", "src/m/a/app/y.ts"),
            ("src/m/a/app/y.ts", "src/m/a/app/x.ts"),
        ],
        "version: 1\nmoves:\n  - { from: \"src/m/<name>/app/**\", to: \"src/app/<name>/\" }\n",
    );
    assert_eq!(
        doc["target"]["moves"],
        json!([{ "slice": "a",
                 "files": [{ "file": "src/m/a/app/x.ts", "to": "src/app/a/x.ts" },
                           { "file": "src/m/a/app/y.ts", "to": "src/app/a/y.ts" }],
                 "import_lines": 2, "names": null }])
    );
}

#[test]
fn default_forbidden_makes_unplanned_imports_forbidden() {
    let base = std::fs::read_to_string(layout("ts-fda")).unwrap();
    let rules = write_tmp(
        "rules/fda-default-forbidden.yaml",
        &format!("{base}default: forbidden\n"),
    );
    let (doc, _, _) = scan_recorded("ts-fda", &["--rules", &rules]);
    assert_eq!(doc["config"]["rules"]["doc"]["default"], "forbidden");
    let clock = verdicts(&doc)
        .into_iter()
        .find(|r| r.1 == "src/lib/clock.ts")
        .unwrap();
    assert_eq!(
        (clock.2.as_str(), clock.3.as_str()),
        ("forbidden", "default")
    );
    assert_eq!(doc["summary"]["forbidden"], 6);
    assert_eq!(doc["summary"]["unplanned"], 0);

    // Without other violations the default alone fails check: six internal imports plus node:fs
    // unplanned imports and exits 0; with `default: forbidden` they are forbidden and it exits 1.
    let plain = scan_file("ts-basic", &[]);
    let strict = write_tmp(
        "rules/empty-default-forbidden.yaml",
        "version: 1\ndefault: forbidden\n",
    );
    let (stdout, _, code) = check(&[&plain, "--rules", &strict]);
    assert_eq!(code, Some(1));
    assert!(stdout.ends_with("7 forbidden, 0 unplanned\n"), "{stdout}");
    assert!(
        stdout.starts_with("forbidden src/app/shell/run.ts -> src/core/model/store.ts (default)\n"),
        "{stdout}"
    );

    assert!(stdout.contains("forbidden src/core/util/log.ts -> pkg:node:fs (default)\n"));

    // Leaving the key out keeps the embedded rules as they were.
    let (doc, _, _) = scan_recorded("ts-fda", &["--rules", &layout("ts-fda")]);
    assert!(doc["config"]["rules"]["doc"].get("default").is_none());
}

#[test]
fn ts_modmono_index_only_even_without_an_index() {
    let (doc, _, _) = scan_recorded("ts-modmono", &["--rules", &layout("ts-modmono")]);
    // `modules: <name>` replaces the depth-2 ids (`modules/a`) for files under src/modules.
    let modules: Vec<(&str, &str)> = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap(), f["module"].as_str().unwrap()))
        .collect();
    assert_eq!(
        modules,
        [
            ("src/main.ts", "main.ts"),
            ("src/modules/a/application/x.ts", "a"),
            ("src/modules/a/application/z.ts", "a"),
            ("src/modules/a/domain/order.ts", "a"),
            ("src/modules/b/domain/y.ts", "b"),
            ("src/modules/c/domain/item.ts", "c"),
            ("src/modules/c/index.ts", "c"),
            ("src/shared/id.ts", "shared"),
        ]
    );
    assert_eq!(
        verdicts(&doc),
        v(&[
            (
                "src/main.ts",
                "src/modules/c/domain/item.ts",
                "allowed",
                "public"
            ),
            (
                "src/modules/a/application/x.ts",
                "src/modules/a/domain/order.ts",
                "allowed",
                "layers"
            ),
            // Trace 4: b has no index.ts, its internals stay private.
            (
                "src/modules/a/application/x.ts",
                "src/modules/b/domain/y.ts",
                "forbidden",
                "public"
            ),
            (
                "src/modules/a/application/z.ts",
                "src/modules/c/domain/item.ts",
                "allowed",
                "public"
            ),
            (
                "src/modules/b/domain/y.ts",
                "src/shared/id.ts",
                "allowed",
                "layers"
            ),
            (
                "src/modules/c/index.ts",
                "src/modules/c/domain/item.ts",
                "allowed",
                "same-module"
            ),
        ])
    );
    assert!(
        !fixture("ts-modmono")
            .join("src/modules/b/index.ts")
            .exists()
    );
    assert_eq!(
        finding(&doc, "internals-reach", "src/modules/b/**")["evidence"],
        json!({ "module": "b", "entries": ["src/modules/b/index.ts"],
                "imports": 1, "importers": 1, "importing_files": 1,
                "edges": [{ "from": "src/modules/a/application/x.ts",
                            "to": "src/modules/b/domain/y.ts" }] })
    );
    let (_, _, code) = check(&[
        &scan_file("ts-modmono", &[]),
        "--rules",
        &layout("ts-modmono"),
    ]);
    assert_eq!(code, Some(1));
}

#[test]
fn ts_hexagonal_inward_layers_and_adapter_siblings() {
    let (doc, _, _) = scan_recorded("ts-hexagonal", &["--rules", &layout("ts-hexagonal")]);
    // Trace 5. Rules without an id are named by position.
    assert_eq!(
        verdicts(&doc),
        v(&[
            (
                "src/adapters/in/http.ts",
                "src/adapters/out/pg.ts",
                "forbidden",
                "forbid[0]"
            ),
            (
                "src/adapters/in/http.ts",
                "src/application/create.ts",
                "allowed",
                "layers"
            ),
            (
                "src/adapters/out/queue.ts",
                "src/adapters/in/cli.ts",
                "forbidden",
                "forbid[1]"
            ),
            (
                "src/application/create.ts",
                "src/domain/user.ts",
                "allowed",
                "layers"
            ),
            (
                "src/domain/user.ts",
                "src/adapters/out/pg.ts",
                "forbidden",
                "layers"
            ),
            (
                "src/domain/user.ts",
                "src/domain/rules.ts",
                "allowed",
                "layers"
            ),
            (
                "src/main.ts",
                "src/adapters/in/http.ts",
                "allowed",
                "layers"
            ),
            ("src/main.ts", "src/adapters/out/pg.ts", "allowed", "layers"),
        ])
    );
}

#[test]
fn py_clean_hexagonal_layers_and_sibling_forbids() {
    let (doc, _, _) = scan_recorded("py-clean", &["--rules", &layout("py-clean")]);
    assert_eq!(doc["config"]["source_root"], "src/app");
    assert_eq!(
        verdicts(&doc),
        v(&[
            (
                "src/app/adapters/notify.py",
                "src/app/entrypoints/cli.py",
                "forbidden",
                "adapters-no-entrypoints"
            ),
            (
                "src/app/adapters/repository.py",
                "src/app/domain/model.py",
                "allowed",
                "layers"
            ),
            (
                "src/app/bootstrap.py",
                "src/app/adapters/repository.py",
                "allowed",
                "layers"
            ),
            (
                "src/app/bootstrap.py",
                "src/app/entrypoints/api.py",
                "allowed",
                "layers"
            ),
            (
                "src/app/domain/events.py",
                "src/app/adapters/repository.py",
                "forbidden",
                "layers"
            ),
            (
                "src/app/entrypoints/api.py",
                "src/app/adapters/repository.py",
                "forbidden",
                "entrypoints-no-adapters"
            ),
            (
                "src/app/entrypoints/api.py",
                "src/app/service_layer/services.py",
                "allowed",
                "layers"
            ),
            (
                "src/app/service_layer/services.py",
                "src/app/domain/model.py",
                "allowed",
                "layers"
            ),
        ])
    );
    let (_, _, code) = check(&[&scan_file("py-clean", &[]), "--rules", &layout("py-clean")]);
    assert_eq!(code, Some(1));
}

#[test]
fn check_fails_only_on_forbidden_imports_missing_from_the_known_file() {
    let rules = write_tmp(
        "known/layout.yaml",
        &std::fs::read_to_string(layout("ts-hexagonal")).unwrap(),
    );
    let known = std::path::Path::new(&rules).with_file_name("layout.known.yaml");
    let _ = std::fs::remove_file(&known);
    let scan = scan_file("ts-hexagonal", &[]);
    let header = "# Forbidden imports layout.yaml already has. check fails on any other, and on an entry that no\n\
                  # longer occurs. Shrink with: boundrail check --update-known shrink\n\
                  version: 1\nknown:\n";
    let http = "  - { rule: \"forbid[0]\", from: \"src/adapters/in/http.ts\", to: \"src/adapters/out/pg.ts\" }\n";
    let queue = "  - { rule: \"forbid[1]\", from: \"src/adapters/out/queue.ts\", to: \"src/adapters/in/cli.ts\" }\n";
    let user =
        "  - { rule: \"layers\", from: \"src/domain/user.ts\", to: \"src/adapters/out/pg.ts\" }\n";
    let all = format!("{header}{http}{queue}{user}");
    let stale = "stale known src/main.ts -> src/adapters/out/pg.ts (forbid[0]): no longer forbidden; \
                 run boundrail check --update-known shrink\n";

    // Two listed: only the third fails.
    std::fs::write(&known, format!("{header}{http}{user}")).unwrap();
    let (stdout, _, code) = check(&[&scan, "--rules", &rules]);
    assert_eq!(code, Some(1));
    assert_eq!(
        stdout,
        "forbidden src/adapters/out/queue.ts -> src/adapters/in/cli.ts (forbid[1])\n\
         3 forbidden (2 known), 0 unplanned\n"
    );

    // All listed: exit 0, the output and the document keep every forbidden import.
    std::fs::write(&known, &all).unwrap();
    let output = write_tmp("known/checked.json", "");
    let (stdout, stderr, code) = check(&[&scan, "--rules", &rules, "-o", &output]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "3 forbidden (3 known), 0 unplanned\n");
    let mut checked: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    validate(&checked);
    assert_eq!(checked["summary"]["forbidden"], 3);
    assert_eq!(checked["summary"]["known"], 3);
    assert_eq!(
        checked["config"]["rules"]["known"]["path"],
        "layout.known.yaml"
    );
    assert_eq!(
        checked["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["known"] == true)
            .count(),
        3
    );
    // Flags read from a document are recomputed from its list, never kept.
    checked["config"]["rules"]["known"] = serde_json::Value::Null;
    let tampered = write_tmp("known/tampered.json", &checked.to_string());
    let (stdout, _, code) = check(&[&tampered]);
    assert_eq!(code, Some(1));
    assert!(stdout.ends_with("\n3 forbidden, 0 unplanned\n"), "{stdout}");

    // An entry that no longer occurs fails until the file shrinks.
    let main =
        "  - { rule: \"forbid[0]\", from: \"src/main.ts\", to: \"src/adapters/out/pg.ts\" }\n";
    std::fs::write(&known, format!("{all}{main}")).unwrap();
    let (stdout, _, code) = check(&[&scan, "--rules", &rules]);
    assert_eq!(code, Some(1));
    assert_eq!(
        stdout,
        format!("{stale}3 forbidden (3 known), 0 unplanned\n")
    );
    let (stdout, stderr, code) = check(&[&scan, "--rules", &rules, "--update-known", "shrink"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stderr.contains("layout.known.yaml: 3 kept, 1 dropped, 0 added\n"),
        "{stderr}"
    );
    assert_eq!(stdout, "3 forbidden (3 known), 0 unplanned\n");
    assert_eq!(std::fs::read_to_string(&known).unwrap(), all);

    // A renamed rule: the old entry is stale and the import unlisted.
    let renamed = "  - { rule: \"layers\", from: \"src/adapters/in/http.ts\", to: \"src/adapters/out/pg.ts\" }\n";
    std::fs::write(&known, format!("{header}{renamed}{queue}{user}")).unwrap();
    let (stdout, _, code) = check(&[&scan, "--rules", &rules]);
    assert_eq!(code, Some(1));
    assert_eq!(
        stdout,
        "forbidden src/adapters/in/http.ts -> src/adapters/out/pg.ts (forbid[0])\n\
         stale known src/adapters/in/http.ts -> src/adapters/out/pg.ts (layers): no longer forbidden; \
         run boundrail check --update-known shrink\n\
         3 forbidden (2 known), 0 unplanned\n"
    );

    // accept creates the file; shrink never does.
    std::fs::remove_file(&known).unwrap();
    let (_, stderr, code) = check(&[&scan, "--rules", &rules, "--update-known", "shrink"]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("no layout.known.yaml"), "{stderr}");
    assert!(!known.exists());
    let (stdout, stderr, code) = check(&[&scan, "--rules", &rules, "--update-known", "accept"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stderr.contains("layout.known.yaml: 0 kept, 0 dropped, 3 added\n"),
        "{stderr}"
    );
    assert_eq!(stdout, "3 forbidden (3 known), 0 unplanned\n");
    assert_eq!(std::fs::read_to_string(&known).unwrap(), all);

    // A saved scan names no rules file to write beside.
    let (_, embedded, _) = scan_recorded("ts-hexagonal", &["--rules", &rules]);
    let embedded = write_tmp("known/embedded.json", &embedded);
    let (_, stderr, code) = check(&[&embedded, "--update-known", "accept"]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("pass --rules"), "{stderr}");

    // Strict like layout.yaml.
    std::fs::write(
        &known,
        "version: 1\nknown:\n  - { rule: \"layers\", from: \"a.ts\", too: \"b.ts\" }\n",
    )
    .unwrap();
    let (_, stderr, code) = check(&[&scan, "--rules", &rules]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("unknown field `too`"), "{stderr}");
    std::fs::remove_file(&known).unwrap();
}

#[test]
fn sarif_marks_known_imports_suppressed_and_fails_only_on_unlisted_ones() {
    let rules = write_tmp(
        "known/sarif-layout.yaml",
        &std::fs::read_to_string(layout("ts-hexagonal")).unwrap(),
    );
    let known = std::path::Path::new(&rules).with_file_name("sarif-layout.known.yaml");
    let scan = scan_file("ts-hexagonal", &[]);
    let http = "  - { rule: \"forbid[0]\", from: \"src/adapters/in/http.ts\", to: \"src/adapters/out/pg.ts\" }\n";
    let queue = "  - { rule: \"forbid[1]\", from: \"src/adapters/out/queue.ts\", to: \"src/adapters/in/cli.ts\" }\n";
    let user =
        "  - { rule: \"layers\", from: \"src/domain/user.ts\", to: \"src/adapters/out/pg.ts\" }\n";
    let suppressed = serde_json::json!([{
        "kind": "external",
        "justification": "Listed in sarif-layout.known.yaml."
    }]);
    let sarif_of = |listed: &str| {
        std::fs::write(&known, format!("version: 1\nknown:\n{listed}")).unwrap();
        let (stdout, _, code) = check(&[&scan, "--rules", &rules, "--format", "sarif"]);
        let sarif: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        validate_sarif(&sarif);
        (sarif["runs"][0]["results"].clone(), code)
    };
    // Two listed: their results carry the suppression, the unlisted one none, exit 1.
    let (results, code) = sarif_of(&format!("{http}{user}"));
    assert_eq!(code, Some(1));
    let rows: Vec<(&str, bool)> = results
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let uri = r["locations"][0]["physicalLocation"]["artifactLocation"]["uri"]
                .as_str()
                .unwrap();
            if r.get("suppressions").is_some() {
                assert_eq!(r["suppressions"], suppressed);
                (uri, true)
            } else {
                (uri, false)
            }
        })
        .collect();
    assert_eq!(rows.len(), 3);
    let marked: Vec<&str> = rows.iter().filter(|r| r.1).map(|r| r.0).collect();
    assert!(
        marked
            .iter()
            .any(|u| u.ends_with("src/adapters/in/http.ts")),
        "{rows:?}"
    );
    assert!(
        marked.iter().any(|u| u.ends_with("src/domain/user.ts")),
        "{rows:?}"
    );
    assert_eq!(marked.len(), 2, "{rows:?}");
    // All listed: every result is suppressed and the exit is 0.
    let (results, code) = sarif_of(&format!("{http}{queue}{user}"));
    assert_eq!(code, Some(0));
    assert!(
        results
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["suppressions"] == suppressed)
    );
    std::fs::remove_file(&known).unwrap();
}

#[test]
fn check_uses_embedded_rules_and_the_flag_replaces_them() {
    let embedded = scan_file("ts-hexagonal", &["--rules", &layout("ts-hexagonal")]);
    let (stdout, _, code) = check(&[&embedded]);
    assert_eq!(code, Some(1));
    assert!(stdout.ends_with("3 forbidden, 0 unplanned\n"), "{stdout}");

    // `*` is one segment: src/* matches src/main.ts, never src/domain/user.ts. user.ts -> rules.ts
    // stays allowed as same-module.
    let star = write_tmp(
        "rules/star.yaml",
        "version: 1\nforbid:\n  - { id: top, from: \"src/*\", to: \"**\" }\n",
    );
    let (stdout, _, code) = check(&[&embedded, "--rules", &star]);
    assert_eq!(code, Some(1));
    assert_eq!(
        stdout,
        "unplanned src/adapters/in/http.ts -> src/adapters/out/pg.ts (none)\n\
         unplanned src/adapters/in/http.ts -> src/application/create.ts (none)\n\
         unplanned src/adapters/out/queue.ts -> src/adapters/in/cli.ts (none)\n\
         unplanned src/application/create.ts -> src/domain/user.ts (none)\n\
         unplanned src/domain/user.ts -> src/adapters/out/pg.ts (none)\n\
         forbidden src/main.ts -> src/adapters/in/http.ts (top)\n\
         forbidden src/main.ts -> src/adapters/out/pg.ts (top)\n\
         2 forbidden, 5 unplanned\n"
    );

    // Nothing forbidden: exit 0. Of ts-basic's 8 edges, view -> run and store -> types stay
    // within a module; the known node:fs import is unplanned.
    let empty = write_tmp("rules/empty.yaml", "version: 1\n");
    let (stdout, _, code) = check(&[&scan_file("ts-basic", &[]), "--rules", &empty]);
    assert_eq!(code, Some(0));
    assert!(stdout.ends_with("0 forbidden, 7 unplanned\n"), "{stdout}");

    assert!(stdout.contains("unplanned src/core/util/log.ts -> pkg:node:fs (none)\n"));

    // No rules anywhere is a usage error.
    let (_, stderr, code) = check(&[&scan_file("ts-basic", &[])]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("no rules"), "{stderr}");
}

#[test]
fn rules_errors() {
    let dir = fixture("ts-hexagonal");
    let input = dir.join("depcruise.json");
    let scan_with = |rules: &str| {
        let out = boundrail(&[
            "scan",
            dir.to_str().unwrap(),
            "--collector-input",
            input.to_str().unwrap(),
            "--rules",
            rules,
        ]);
        (out.status.code(), String::from_utf8(out.stderr).unwrap())
    };

    // A typo cannot silently allow everything.
    let typo = write_tmp(
        "rules/typo.yaml",
        "version: 1\nforbid:\n  - { from: \"src/domain/**\", too: \"src/adapters/**\" }\n",
    );
    let (code, stderr) = scan_with(&typo);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("unknown field `too`"), "{stderr}");

    let top = write_tmp("rules/top.yaml", "version: 1\nlayer: []\n");
    let (code, stderr) = scan_with(&top);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("unknown field `layer`"), "{stderr}");

    let default = write_tmp("rules/default-bad.yaml", "version: 1\ndefault: deny\n");
    let (code, stderr) = scan_with(&default);
    assert_eq!(code, Some(2));
    assert!(
        stderr.contains("deny") && stderr.contains("forbidden"),
        "{stderr}"
    );
    let default = write_tmp("rules/default-ok.yaml", "version: 1\ndefault: forbidden\n");
    let (code, stderr) = scan_with(&default);
    assert_eq!(code, Some(0), "{stderr}");

    // A blank id or reason would print as nothing in check, review and SARIF, which GitHub rejects.
    for (name, yaml, field) in [
        (
            "rules/blank-forbid-id.yaml",
            "version: 1\nforbid:\n  - { id: \" \", from: \"src/domain/**\", to: \"src/adapters/**\" }\n",
            "forbid[0].id",
        ),
        (
            "rules/blank-forbid-reason.yaml",
            "version: 1\nforbid:\n  - { id: a, from: \"src/domain/**\", to: \"src/adapters/**\", reason: \"\" }\n",
            "forbid[0].reason",
        ),
        (
            "rules/blank-allow-id.yaml",
            "version: 1\nallow:\n  - { id: \"\", from: \"src/**\", to: \"src/**\" }\n",
            "allow[0].id",
        ),
        (
            "rules/blank-allow-reason.yaml",
            "version: 1\nallow:\n  - { from: \"src/**\", to: \"src/**\", reason: \"  \" }\n",
            "allow[0].reason",
        ),
    ] {
        let (code, stderr) = scan_with(&write_tmp(name, yaml));
        assert_eq!(code, Some(2), "{name}: {stderr}");
        assert!(
            stderr.contains(&format!("{field} must be nonempty")),
            "{name}: {stderr}"
        );
    }

    let version = write_tmp("rules/version.yaml", "version: 2\n");
    let (code, stderr) = scan_with(&version);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("version"), "{stderr}");

    // Rule ids, reasons and module ids are printed by report and check and land in CI logs and
    // job summaries: a line break there could forge a workflow command such as `::error::`.
    for (name, yaml) in [
        (
            "rules/control-id.yaml",
            "version: 1\nforbid:\n  - { id: \"a\\n::error::forged\", from: \"src/domain/**\", to: \"src/adapters/**\" }\n",
        ),
        (
            "rules/control-reason.yaml",
            "version: 1\nallow:\n  - { from: \"src/**\", to: \"src/**\", reason: \"ok\\r\" }\n",
        ),
        (
            "rules/control-module.yaml",
            "version: 1\nmodules:\n  - { id: \"core\\tx\", path: \"src/domain/**\" }\n",
        ),
    ] {
        let (code, stderr) = scan_with(&write_tmp(name, yaml));
        assert_eq!(code, Some(2), "{name}: {stderr}");
        assert!(stderr.contains("control character"), "{name}: {stderr}");
        assert!(!stderr.contains("\n::error::forged"), "{name}: {stderr}");
    }

    // A capture must be a whole segment, and entries can only use the module's capture.
    let partial = write_tmp(
        "rules/partial.yaml",
        "version: 1\nlayers: [\"src/<name>.ts\"]\n",
    );
    let (code, stderr) = scan_with(&partial);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("whole path segment"), "{stderr}");
    let unbound = write_tmp(
        "rules/unbound.yaml",
        "version: 1\npublic:\n  - { module: \"src/<a>/**\", entries: [\"src/<b>/index.ts\"] }\n",
    );
    let (code, stderr) = scan_with(&unbound);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("<b>"), "{stderr}");

    // A move's `to` is one folder per slice, written without globs.
    for (name, rule, expected) in [
        (
            "glob",
            "{ from: \"src/<name>/*.ts\", to: \"lib/<name>/*/\" }",
            "moves[0].to must be a folder ending in `/` without globs",
        ),
        (
            "slash",
            "{ from: \"src/<name>/*.ts\", to: \"lib/<name>\" }",
            "moves[0].to must be a folder ending in `/` without globs",
        ),
        (
            "capture",
            "{ from: \"src/<name>/*.ts\", to: \"lib/<other>/\" }",
            "`lib/<other>/` uses <other>, which `src/<name>/*.ts` does not bind",
        ),
        (
            "pkg",
            "{ from: \"pkg:x\", to: \"lib/\" }",
            "moves[0].from must be a file glob",
        ),
        (
            "key",
            "{ from: \"src/<name>/*.ts\", to: \"lib/<name>/\", into: \"x\" }",
            "unknown field `into` in moves[0]",
        ),
    ] {
        let path = write_tmp(
            &format!("rules/moves-{name}.yaml"),
            &format!("version: 1\nmoves:\n  - {rule}\n"),
        );
        let (code, stderr) = scan_with(&path);
        assert_eq!(code, Some(2), "{name}: {stderr}");
        assert!(stderr.contains(expected), "{name}: {stderr}");
    }
}

/// ts-modmono's layout plus `extra` lines.
fn modmono_with(name: &str, extra: &str) -> String {
    let base = std::fs::read_to_string(layout("ts-modmono")).unwrap();
    write_tmp(&format!("rules/{name}.yaml"), &format!("{base}{extra}"))
}

#[test]
fn a_capture_binds_from_and_to_of_allow_and_forbid() {
    // `<name>` is the same module on both sides, so a -> b is not "within one module".
    let allow = modmono_with(
        "bound-allow",
        "allow:\n  - { from: \"src/modules/<name>/**\", to: \"src/modules/<name>/**\" }\n",
    );
    let (doc, _, _) = scan_recorded("ts-modmono", &["--rules", &allow]);
    let rows = verdicts(&doc);
    let verdict = |from: &str, to: &str| {
        rows.iter()
            .find(|r| r.0 == from && r.1 == to)
            .map(|r| (r.2.as_str(), r.3.as_str()))
            .unwrap()
    };
    assert_eq!(
        verdict(
            "src/modules/a/application/x.ts",
            "src/modules/b/domain/y.ts"
        ),
        ("forbidden", "public")
    );
    assert_eq!(
        verdict(
            "src/modules/a/application/x.ts",
            "src/modules/a/domain/order.ts"
        ),
        ("allowed", "allow[0]")
    );

    // A bound forbid fires only when both files are in the same module instance.
    let forbid = modmono_with(
        "bound-forbid",
        "forbid:\n  - { id: app-no-own-domain, from: \"src/modules/<name>/application/**\", \
         to: \"src/modules/<name>/domain/**\" }\n",
    );
    let (doc, _, _) = scan_recorded("ts-modmono", &["--rules", &forbid]);
    let rows = verdicts(&doc);
    let verdict = |from: &str, to: &str| {
        rows.iter()
            .find(|r| r.0 == from && r.1 == to)
            .map(|r| (r.2.as_str(), r.3.as_str()))
            .unwrap()
    };
    assert_eq!(
        verdict(
            "src/modules/a/application/x.ts",
            "src/modules/a/domain/order.ts"
        ),
        ("forbidden", "app-no-own-domain")
    );
    assert_eq!(
        verdict(
            "src/modules/a/application/x.ts",
            "src/modules/b/domain/y.ts"
        ),
        ("forbidden", "public")
    );
}

#[test]
fn captures_are_rejected_where_nothing_binds_them() {
    let dir = fixture("ts-modmono");
    let input = dir.join("depcruise.json");
    let scan_with = |rules: &str| {
        let out = boundrail(&[
            "scan",
            dir.to_str().unwrap(),
            "--collector-input",
            input.to_str().unwrap(),
            "--rules",
            rules,
        ]);
        (out.status.code(), String::from_utf8(out.stderr).unwrap())
    };
    let layers = write_tmp(
        "rules/capture-layer.yaml",
        "version: 1\nlayers: [\"src/modules/<name>/domain/**\"]\n",
    );
    let (code, stderr) = scan_with(&layers);
    assert_eq!(code, Some(2));
    assert!(
        stderr.contains("layers") && stderr.contains("`*`"),
        "{stderr}"
    );

    let to_only = write_tmp(
        "rules/capture-to.yaml",
        "version: 1\nforbid:\n  - { from: \"src/**\", to: \"src/modules/<name>/**\" }\n",
    );
    let (code, stderr) = scan_with(&to_only);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("<name>"), "{stderr}");
}

#[test]
fn rules_embedded_in_a_scan_may_carry_unknown_fields() {
    // A newer boundrail may add rule keys; reading its scan keeps working. Only layout.yaml is
    // strict.
    let (mut doc, _, _) = scan_recorded("ts-hexagonal", &["--rules", &layout("ts-hexagonal")]);
    doc["config"]["rules"]["doc"]["future"] = json!(true);
    doc["config"]["rules"]["doc"]["forbid"][0]["severity"] = json!("high");
    let graph = write_tmp("rules/future-keys.json", &doc.to_string());
    let (stdout, stderr, code) = check(&[&graph]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stdout.ends_with("3 forbidden, 0 unplanned\n"), "{stdout}");
}

#[test]
fn inconsistent_scan_documents_are_input_errors() {
    let (doc, _, _) = scan_recorded("ts-basic", &[]);
    let broken = |name: &str, edit: &dyn Fn(&mut serde_json::Value)| {
        let mut bad = doc.clone();
        edit(&mut bad);
        let path = write_tmp(&format!("bad/{name}.json"), &bad.to_string());
        let mut results = Vec::new();
        for args in [
            vec!["render", path.as_str()],
            vec!["diff", path.as_str(), path.as_str()],
            vec![
                "simulate",
                path.as_str(),
                "--move",
                "src/main.ts",
                "src/x/main.ts",
            ],
        ] {
            let out = boundrail(&args);
            let stderr = String::from_utf8(out.stderr).unwrap();
            assert_eq!(out.status.code(), Some(2), "{name} {args:?}: {stderr}");
            assert!(!stderr.contains("panicked"), "{name}: {stderr}");
            results.push(stderr);
        }
        results.swap_remove(0)
    };
    let stderr = broken("ghost-edge", &|d| {
        d["edges"].as_array_mut().unwrap().push(json!({
            "from": "src/main.ts", "to": "src/ghost.ts", "kind": "runtime", "weight": 1.0, "verdict": null,
            "rule": null }))
    });
    assert!(stderr.contains("src/ghost.ts"), "{stderr}");
    let stderr = broken("duplicate-file", &|d| {
        let first = d["files"][0].clone();
        d["files"].as_array_mut().unwrap().push(first);
    });
    assert!(stderr.contains("src/app/shell/run.ts"), "{stderr}");
    let stderr = broken("wrong-module", &|d| {
        d["files"][0]["module"] = json!("elsewhere")
    });
    assert!(stderr.contains("elsewhere"), "{stderr}");
}
