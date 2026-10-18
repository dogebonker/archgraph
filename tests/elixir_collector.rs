mod common;

use std::path::Path;

use archgraph::collect::{
    elixir,
    excludes::{Options, Policy},
};
use archgraph::model::{EdgeKind, ImportReason, Language};
use serde_json::{Value, json};

fn policy(root: &Path) -> Policy {
    Policy::new(root, Language::Elixir, Options::default()).unwrap()
}

fn runtime_available() -> bool {
    std::process::Command::new("elixir")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

fn project(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let first = common::write_tmp(&format!("elixir-{name}-{nonce}/{}", files[0].0), files[0].1);
    let mut root = Path::new(&first).to_path_buf();
    for _ in Path::new(files[0].0).components() {
        root.pop();
    }
    for (path, source) in &files[1..] {
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, source).unwrap();
    }
    root
}

#[test]
fn elixir_records_validate_paths_lines_membership_and_resolution_outcomes() {
    let valid = json!({"elixir_version":"1.14", "files":["lib/a.ex","lib/b.ex"],
        "edges":[{"from":"lib/a.ex","to":"lib/b.ex","kind":"runtime","line":2}],
        "imports":[{"from":"lib/a.ex","specifier":"B","kind":"runtime","line":2,"targets":["lib/b.ex"]}]});
    assert!(elixir::parse(&serde_json::to_vec(&valid).unwrap()).is_ok());
    for path in [
        "../a.ex", "/a.ex", "C:/a.ex", "a\\b.ex", "a//b.ex", "./a.ex", "a/./b.ex",
    ] {
        let mut invalid = valid.clone();
        invalid["files"][0] = json!(path);
        assert!(
            elixir::parse(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "{path}"
        );
    }
    for (pointer, replacement) in [
        ("/files/1", json!("lib/a.ex")),
        ("/edges/0/to", json!("lib/unknown.ex")),
        ("/edges/0/line", json!(0)),
        ("/imports/0/line", Value::Null),
        ("/imports/0/targets/0", json!("../b.ex")),
        ("/imports/0/from", json!("unknown.ex")),
        ("/imports/0/specifier", json!("")),
        ("/imports/0/targets", json!([])),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            elixir::parse(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "{pointer}"
        );
    }
    let mut ambiguous = valid;
    ambiguous["imports"][0]["external"] = json!("ecto");
    assert!(elixir::parse(&serde_json::to_vec(&ambiguous).unwrap()).is_err());
    assert!(elixir::parse(b"not JSON").is_err());
}

#[test]
fn elixir_live_tracks_group_aliases_types_structs_captures_and_runtime_apps() {
    if !runtime_available() {
        eprintln!("Elixir unavailable; live check skipped");
        return;
    }
    let root = project(
        "aliases",
        &[
            (
                "lib/client.ex",
                r#"
        defmodule Sample.Client do
          alias Sample.{Order, Store}
          alias Sample.Unused
          @spec run(Order.t()) :: Store.t()
          def run do
            Store.save(%Order{})
            &Store.save/1
            Enum.map([], & &1)
            :lists.reverse([])
          end
        end
        "#,
            ),
            ("lib/order.ex", "defmodule Sample.Order do end"),
            ("lib/store.ex", "defmodule Sample.Store do end"),
        ],
    );
    let raw = elixir::parse(&elixir::run(&root, "elixir", &policy(&root)).unwrap()).unwrap();
    assert_eq!(raw.collector.name, "elixir-ast");
    for target in ["lib/order.ex", "lib/store.ex"] {
        assert!(
            raw.edges
                .iter()
                .any(|e| e.to == target && e.kind == EdgeKind::Type)
        );
        assert!(
            raw.edges
                .iter()
                .any(|e| e.to == target && e.kind == EdgeKind::Static)
        );
    }
    let imports = raw.imports.unwrap();
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "Enum" && i.external.as_deref() == Some("elixir"))
    );
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == ":lists" && i.external.as_deref() == Some("stdlib"))
    );
    assert!(!imports.iter().any(|i| i.specifier.contains("Unused")));
    assert_eq!(raw.collector.unresolved, Some(0));
}

#[test]
fn elixir_live_aliases_are_lexical_and_nested_modules_resolve_across_files() {
    if !runtime_available() {
        eprintln!("Elixir unavailable; live check skipped");
        return;
    }
    let root = project(
        "lexical",
        &[
            (
                "lib/outer.ex",
                r#"
        defmodule Outer do
          defmodule Inner do
            def run, do: __MODULE__.run()
          end
          Inner.run()
          __MODULE__.Inner.run()
          def first do
            alias Actual.One, as: Selected
            Selected.run()
          end
          def second do
            alias Actual.Two, as: Selected
            Selected.run()
          end
          def third, do: Selected.run()
        end
        "#,
            ),
            ("lib/one.ex", "defmodule Actual.One do end"),
            ("lib/two.ex", "defmodule Actual.Two do end"),
            ("lib/client.ex", "defmodule Client do Outer.Inner.run() end"),
            ("lib/duplicate.ex", "defmodule Actual.One do end"),
        ],
    );
    let raw = elixir::parse(&elixir::run(&root, "elixir", &policy(&root)).unwrap()).unwrap();
    assert!(
        raw.edges
            .iter()
            .any(|e| e.from == "lib/client.ex" && e.to == "lib/outer.ex")
    );
    for target in ["lib/one.ex", "lib/two.ex", "lib/duplicate.ex"] {
        assert!(
            raw.edges
                .iter()
                .any(|e| e.from == "lib/outer.ex" && e.to == target)
        );
    }
    let imports = raw.imports.unwrap();
    assert_eq!(
        imports
            .iter()
            .filter(|i| i.specifier == "Outer.Inner")
            .count(),
        4
    );
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "Selected"
                && matches!(i.unresolved, Some(ImportReason::Missing)))
    );
}

#[test]
fn elixir_live_reports_missing_and_dynamic_without_guessing_or_scanning_quotes() {
    if !runtime_available() {
        eprintln!("Elixir unavailable; live check skipped");
        return;
    }
    let root = project(
        "unresolved",
        &[(
            "lib/source.ex",
            r#"
        defmodule Source do
          import Missing.Import
          require Missing.Require
          use Ecto.Schema
          def run(receiver) do
            receiver.run()
            Module.concat([Unknown, Name])
            quote do: NotReal.run()
            "AlsoNotReal.run()"
            # CommentNotReal.run()
          end
        end
        "#,
        )],
    );
    let raw = elixir::parse(&elixir::run(&root, "elixir", &policy(&root)).unwrap()).unwrap();
    let imports = raw.imports.unwrap();
    for name in ["Missing.Import", "Missing.Require", "Ecto.Schema"] {
        assert!(imports.iter().any(|i| i.specifier == name && matches!(i.unresolved, Some(ImportReason::Missing))));
    }
    assert!(
        imports
            .iter()
            .filter(|i| matches!(i.unresolved, Some(ImportReason::Dynamic)))
            .count()
            >= 2
    );
    assert!(!imports.iter().any(|i| i.specifier.contains("NotReal")));
    assert!(
        !imports
            .iter()
            .any(|i| i.external.as_deref() == Some("ecto"))
    );
}

#[test]
fn elixir_live_never_evaluates_project_code_and_keeps_unparsable_files_with_a_warning() {
    if !runtime_available() {
        eprintln!("Elixir unavailable; live check skipped");
        return;
    }
    let root = project(
        "no-execution",
        &[
            ("mix.exs", "raise \"must not execute mix.exs\""),
            (
                "lib/source.ex",
                "defmodule Poison do\n raise \"must not execute module body\"\nend",
            ),
            (".iex.exs", "raise \"must not execute iex startup\""),
        ],
    );
    assert!(elixir::run(&root, "elixir", &policy(&root)).is_ok());
    std::fs::write(
        root.join("lib/broken.ex"),
        "defmodule Broken do\n  def run(\nend",
    )
    .unwrap();
    // Mix templates (`<%= %>`) are not Elixir syntax either: one such file cannot abort a scan.
    std::fs::write(
        root.join("lib/template.exs"),
        "defmodule <%= @module %>.Repo do end",
    )
    .unwrap();
    let (doc, _, stderr) = common::scan(&[root.to_str().unwrap(), "--language", "elixir"]);
    assert!(stderr.contains("warning: lib/broken.ex:2: "), "{stderr}");
    assert!(stderr.contains("warning: lib/template.exs:1: "), "{stderr}");
    let files: Vec<_> = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        files,
        [
            ".iex.exs",
            "lib/broken.ex",
            "lib/source.ex",
            "lib/template.exs",
            "mix.exs"
        ]
    );
}

#[test]
fn elixir_live_discovery_applies_excludes_and_does_not_follow_symlinks() {
    if !runtime_available() {
        eprintln!("Elixir unavailable; live check skipped");
        return;
    }
    let root = project(
        "discovery",
        &[
            ("lib/source.ex", "defmodule Source do end"),
            ("deps/bad.ex", "invalid source !!!"),
            (".git/bad.ex", "invalid source !!!"),
            ("node_modules/pkg/bad.ex", "invalid source !!!"),
            ("nested/test.exs", "defmodule Nested do Source.run() end"),
        ],
    );
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("deps/bad.ex"), root.join("lib/link.ex")).unwrap();
    let raw = elixir::parse(&elixir::run(&root, "elixir", &policy(&root)).unwrap()).unwrap();
    assert_eq!(
        raw.files,
        ["lib/source.ex", "nested/test.exs", "deps/bad.ex"]
    );
    assert_eq!(raw.collector.cruised, 2);
}

#[test]
fn elixir_live_selection_precedes_parsing_ignored_tests_and_generated_sources() {
    if !runtime_available() {
        eprintln!("Elixir unavailable; live check skipped");
        return;
    }
    let root = project(
        "selection",
        &[
            ("lib/source.ex", "defmodule Source do end"),
            ("test/broken_test.exs", "defmodule Test do @not valid !!!"),
            ("_build/broken.ex", "defmodule Generated do @not valid !!!"),
            ("ignored/broken.ex", "defmodule Ignored do @not valid !!!"),
            (".archgraphignore", "ignored/\n"),
        ],
    );
    let raw = elixir::parse(&elixir::run(&root, "elixir", &policy(&root)).unwrap()).unwrap();
    assert_eq!(raw.collector.cruised, 1);
    for options in [
        Options {
            include_tests: true,
            ..Options::default()
        },
        Options {
            include_generated: true,
            ..Options::default()
        },
        Options {
            includes: vec!["ignored/**".into()],
            ..Options::default()
        },
    ] {
        let selected = Policy::new(&root, Language::Elixir, options).unwrap();
        let raw = elixir::parse(&elixir::run(&root, "elixir", &selected).unwrap()).unwrap();
        assert_eq!(raw.collector.cruised, 2);
    }
}

#[test]
fn elixir_cli_sanitizes_startup_flags_and_project_code_paths() {
    if !runtime_available() {
        eprintln!("Elixir unavailable; live check skipped");
        return;
    }
    let root = project(
        "startup",
        &[
            (
                "lib/source.ex",
                "defmodule Source do Injected.Module.run() end",
            ),
            (
                "deps/evil/ebin/evil.app",
                "{application, evil, [{modules, ['Elixir.Injected.Module']}]} .",
            ),
        ],
    );
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .args(["scan", root.to_str().unwrap(), "--language", "elixir"])
        .env("ELIXIR_ERL_OPTIONS", "-not_a_real_runtime_option")
        .env("ELIXIR_ERL_PARAMETERS", "-not_a_real_runtime_option")
        .env("ERL_AFLAGS", "-not_a_real_runtime_option")
        .env("ERL_FLAGS", "-not_a_real_runtime_option")
        .env("ERL_ZFLAGS", "-not_a_real_runtime_option")
        .env("ERL_LIBS", root.join("deps"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["import_resolution"]["unresolved"], 1);
    assert!(doc["external_nodes"].as_array().is_none_or(Vec::is_empty));
    common::validate(&doc);
}

#[test]
fn elixir_live_definition_aliases_match_lexical_nested_module_semantics() {
    if !runtime_available() {
        eprintln!("Elixir unavailable; live check skipped");
        return;
    }
    let root = project(
        "definition-alias",
        &[
            (
                "lib/top.ex",
                r#"
          defmodule Namespace.Child do end
          Child.run()
          defmodule Outer do
            defmodule A.B do
              A.B.run()
            end
            A.B.run()
            B.run()
            defmodule Elixir.Namespace.Defined do end
            Defined.run()
          end
        "#,
            ),
            ("lib/child.ex", "defmodule Child do end"),
            ("lib/b.ex", "defmodule B do end"),
            ("lib/defined.ex", "defmodule Defined do end"),
        ],
    );
    let raw = elixir::parse(&elixir::run(&root, "elixir", &policy(&root)).unwrap()).unwrap();
    for target in ["lib/child.ex", "lib/b.ex", "lib/defined.ex"] {
        assert!(
            raw.edges
                .iter()
                .any(|e| e.from == "lib/top.ex" && e.to == target)
        );
    }
    let imports = raw.imports.unwrap();
    assert_eq!(
        imports
            .iter()
            .filter(|i| i.specifier == "Outer.A.B" && i.targets == ["lib/top.ex"])
            .count(),
        2
    );
    assert_eq!(raw.collector.unresolved, Some(0));
}

#[test]
fn elixir_live_deduplicates_edges_without_dropping_occurrences_or_inventing_data_modules() {
    if !runtime_available() {
        eprintln!("Elixir unavailable; live check skipped");
        return;
    }
    let root = project(
        "occurrences",
        &[
            (
                "lib/source.ex",
                r#"
          defmodule Source do
            @behaviour Missing.Behaviour
            @behaviour :gen_server
            def run do
              Target.run()
              Target.run()
              Target.run(); Target.run()
              :ets.new(NotAModule.Keys, [:named_table])
              %Missing.Struct{}
              Missing.Module.run()
              [Target]
            end
          end
        "#,
            ),
            ("lib/target.ex", "defmodule Target do end"),
        ],
    );
    let raw = elixir::parse(&elixir::run(&root, "elixir", &policy(&root)).unwrap()).unwrap();
    assert_eq!(
        raw.edges
            .iter()
            .filter(|e| e.from == "lib/source.ex"
                && e.to == "lib/target.ex"
                && e.kind == EdgeKind::Static)
            .count(),
        1
    );
    let imports = raw.imports.unwrap();
    assert_eq!(
        imports.iter().filter(|i| i.specifier == "Target").count(),
        5
    );
    assert!(!imports.iter().any(|i| i.specifier == "NotAModule.Keys"));
    for module in ["Missing.Struct", "Missing.Module", "Missing.Behaviour"] {
        assert!(
            imports
                .iter()
                .any(|i| i.specifier == module
                    && matches!(i.unresolved, Some(ImportReason::Missing)))
        );
    }
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == ":gen_server" && i.external.as_deref() == Some("stdlib"))
    );
}

#[test]
fn elixir_live_concat_namespaces_do_not_create_false_parent_cycles() {
    if !runtime_available() {
        eprintln!("Elixir unavailable; live check skipped");
        return;
    }
    let root = project(
        "concat",
        &[
            (
                "lib/parent.ex",
                "defmodule Parent do Store.run(\"memory\") end",
            ),
            (
                "lib/store.ex",
                r#"
          defmodule Store do
            alias Module, as: M
            def run(reference) do
              M.concat(Parent, String.upcase(reference))
              Module.concat([Parent, "Child"])
              Module.safe_concat(__MODULE__, "Child")
            end
          end
        "#,
            ),
            ("lib/custom.ex", "defmodule Custom do end"),
            (
                "lib/shadow.ex",
                "defmodule Shadow do alias Custom, as: Module; Module.concat(Parent) end",
            ),
        ],
    );
    let raw = elixir::parse(&elixir::run(&root, "elixir", &policy(&root)).unwrap()).unwrap();
    assert!(
        raw.edges
            .iter()
            .any(|e| e.from == "lib/parent.ex" && e.to == "lib/store.ex")
    );
    assert!(
        !raw.edges
            .iter()
            .any(|e| e.from == "lib/store.ex" && e.to == "lib/parent.ex")
    );
    for target in ["lib/custom.ex", "lib/parent.ex"] {
        assert!(
            raw.edges
                .iter()
                .any(|e| e.from == "lib/shadow.ex" && e.to == target)
        );
    }
    let imports = raw.imports.unwrap();
    assert_eq!(
        imports
            .iter()
            .filter(
                |i| i.from == "lib/store.ex" && matches!(i.unresolved, Some(ImportReason::Dynamic))
            )
            .count(),
        3
    );
    assert!(imports.iter().any(|i| i.from == "lib/store.ex"
        && i.specifier == "String"
        && i.external.as_deref() == Some("elixir")));
    assert!(
        !imports.iter().any(
            |i| i.from == "lib/store.ex" && ["Parent", "Store"].contains(&i.specifier.as_str())
        )
    );
}

#[test]
fn elixir_live_explicit_global_alias_names_resolve_and_dynamic_alias_forms_do_not_crash() {
    if !runtime_available() {
        eprintln!("Elixir unavailable; live check skipped");
        return;
    }
    let root = project(
        "global-as",
        &[
            (
                "lib/source.ex",
                r#"
          defmodule Source do
            alias Actual.Target, as: Elixir.Short
            Short.run()
            alias Actual.Target, as: __MODULE__.Dynamic
            Dynamic.run()
          end
        "#,
            ),
            ("lib/target.ex", "defmodule Actual.Target do end"),
        ],
    );
    let raw = elixir::parse(&elixir::run(&root, "elixir", &policy(&root)).unwrap()).unwrap();
    assert!(
        raw.edges
            .iter()
            .any(|e| e.from == "lib/source.ex" && e.to == "lib/target.ex")
    );
    let imports = raw.imports.unwrap();
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "Actual.Target" && i.targets == ["lib/target.ex"])
    );
    assert!(
        imports.iter().any(
            |i| i.specifier == "Dynamic" && matches!(i.unresolved, Some(ImportReason::Missing))
        )
    );
}
