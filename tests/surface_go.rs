mod common;

use archgraph::{
    model::{Config, Source},
    symbols::golang,
};
use std::{fs, path::Path};

fn fixture(name: &str, consumer: &str) -> (String, archgraph::model::Document) {
    let (root, _input) = common::synthetic(name, 2, &[("app/app.go".into(), "api/api.go".into())]);
    let root_path = Path::new(&root);
    fs::create_dir_all(root_path.join("api")).unwrap();
    fs::create_dir_all(root_path.join("app")).unwrap();
    fs::write(
        root_path.join("go.mod"),
        "module example.com/surface\n\ngo 1.21\n",
    )
    .unwrap();
    fs::write(root_path.join("api/api.go"), "package api\nfunc Used() {}\nfunc Unused() {}\ntype Item struct{ Value int; Unused int }\ntype Alias = Item\ntype Contract interface { Ping() }\ntype Box[T any] struct{ Value T }\nfunc (Item) Touch() {}\nfunc private() {}\nfunc init() { panic(\"project code executed\") }\n").unwrap();
    fs::write(root_path.join("app/app.go"), consumer).unwrap();
    let source: Source = serde_json::from_value(serde_json::json!({
        "language": "go", "collector": {"name": "go-types", "version": null, "cruised": 2},
        "git": null, "generated_at": "2026-10-17T00:00:00Z"
    }))
    .unwrap();
    let config: Config = serde_json::from_value(serde_json::json!({
        "depth": 1, "source_root": ".", "package_modules": true,
        "excludes": [], "include_tests": false, "rules": null
    }))
    .unwrap();
    let doc = archgraph::analyze(
        source,
        config,
        vec!["api/api.go".into(), "app/app.go".into()],
        Vec::new(),
        Vec::new(),
        None,
    )
    .unwrap();
    (root, doc)
}

#[test]
#[ignore = "requires Go 1.21 or later"]
fn go_surface_tracks_aliases_dot_imports_and_methods_without_running_project_code() {
    let (root, doc) = fixture(
        "surface-go-uses",
        "package app\nimport renamed \"example.com/surface/api\"\nfunc Run() { renamed.Used(); renamed.Item{}.Touch(); _ = renamed.Alias{Value: 1}.Value; _ = renamed.Box[int]{Value: 2}.Value }\nfunc Via(c renamed.Contract) { c.Ping() }\n",
    );
    #[cfg(unix)]
    let go = {
        use std::os::unix::fs::PermissionsExt;
        let wrapper = Path::new(&root).join("safe-go");
        fs::write(&wrapper, "#!/bin/sh\n[ \"$GOENV\" = off ] && [ -z \"$GOCACHEPROG\" ] && [ \"$GOTOOLCHAIN\" = local ] && [ \"$GOPROXY\" = off ] && [ \"$GOFLAGS\" = -mod=readonly ] || exit 42\nexec go \"$@\"\n").unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        wrapper.to_string_lossy().into_owned()
    };
    #[cfg(not(unix))]
    let go = "go".to_owned();
    let symbols = golang::collect(Path::new(&root), &doc, &go).unwrap();
    assert!(symbols.complete, "{:?}", symbols.limitations);
    for name in [
        "Used",
        "Unused",
        "Item",
        "Item.Touch",
        "Item.Value",
        "Item.Unused",
        "Box.Value",
        "Contract.Ping",
    ] {
        assert!(
            symbols
                .exports
                .iter()
                .any(|s| s.file == "api/api.go" && s.name == name)
        );
    }
    assert!(!symbols.exports.iter().any(|s| s.name == "private"));
    for name in [
        "Used",
        "Item",
        "Item.Touch",
        "Item.Value",
        "Box.Value",
        "Contract.Ping",
    ] {
        assert!(
            symbols
                .imports
                .iter()
                .any(|s| s.from == "app/app.go" && s.to == "api/api.go" && s.name == name)
        );
    }
    assert!(
        !symbols
            .imports
            .iter()
            .any(|s| s.name == "Unused" || s.name == "Item.Unused")
    );
    fs::write(
        Path::new(&root).join("app/app.go"),
        "package app\nimport . \"example.com/surface/api\"\nfunc Run() { Used() }\n",
    )
    .unwrap();
    let dot = golang::collect(Path::new(&root), &doc, "go").unwrap();
    assert!(dot.complete);
    assert!(
        dot.imports
            .iter()
            .any(|s| s.name == "Used" && s.to == "api/api.go")
    );
}

#[test]
#[ignore = "requires Go 1.21 or later"]
fn incomplete_go_types_are_a_data_gap() {
    let (root, doc) = fixture(
        "surface-go-gap",
        "package app\nimport missing \"example.com/not-cached\"\nfunc Run() { missing.F() }\n",
    );
    let symbols = golang::collect(Path::new(&root), &doc, "go").unwrap();
    assert!(!symbols.complete);
    assert!(symbols.limitations.iter().any(|s| s.contains("not-cached")));
}

#[test]
#[ignore = "requires Go 1.21 or later"]
fn anonymous_struct_fields_withhold_unused_api_suggestions() {
    let (root, doc) = fixture(
        "surface-go-anonymous-field",
        "package app\nimport api \"example.com/surface/api\"\nfunc Run() { _ = api.Config.Value }\n",
    );
    let api = Path::new(&root).join("api/api.go");
    let mut source = fs::read_to_string(&api).unwrap();
    source.push_str("var Config struct { Value int }\n");
    fs::write(api, source).unwrap();
    let symbols = golang::collect(Path::new(&root), &doc, "go").unwrap();
    assert!(!symbols.complete);
    assert!(
        symbols
            .limitations
            .iter()
            .any(|gap| gap.contains("anonymous structs"))
    );
    assert!(symbols.imports.iter().any(|symbol| symbol.name == "Config"));
}

#[test]
#[ignore = "requires Go 1.21 or later"]
fn factory_values_keep_methods_used_through_local_interfaces() {
    let (root, doc) = fixture(
        "surface-go-interface-dispatch",
        "package app\nimport api \"example.com/surface/api\"\ntype Reader interface { Read([]byte) (int, error) }\nvar R Reader = api.New()\nfunc Run() { R.Read(nil) }\n",
    );
    fs::write(Path::new(&root).join("api/api.go"), "package api\ntype Value struct{}\nfunc New() Value { return Value{} }\nfunc (Value) Read([]byte) (int,error) { return 0,nil }\n").unwrap();
    let symbols = golang::collect(Path::new(&root), &doc, "go").unwrap();
    assert!(symbols.complete, "{:?}", symbols.limitations);
    assert!(
        symbols
            .imports
            .iter()
            .any(|symbol| symbol.from == "app/app.go" && symbol.name == "Value.Read")
    );
    assert!(
        symbols
            .limitations
            .iter()
            .any(|gap| gap.contains("conservative"))
    );
}

#[test]
#[ignore = "requires Go 1.21 or later"]
fn public_interface_factories_preserve_concrete_implementation_methods() {
    for (case, returned, imports) in [
        ("named", "Reader", ""),
        ("anonymous", "interface { Read([]byte) (int,error) }", ""),
        ("standard", "io.Reader", "import \"io\"\n"),
    ] {
        for receiver in ["Value", "value"] {
            let (root, mut doc) = fixture(
                &format!("surface-go-public-interface-{case}-{receiver}"),
                "package app\nimport api \"example.com/surface/api\"\nvar R = api.New()\nfunc Run() { R.Read(nil) }\n",
            );
            fs::write(Path::new(&root).join("api/api.go"), format!("package api\n{imports}type Reader interface {{ Read([]byte) (int,error) }}\ntype {receiver} struct{{}}\nfunc New() {returned} {{ return {receiver}{{}} }}\nfunc ({receiver}) Read([]byte) (int,error) {{ return 0,nil }}\nfunc ({receiver}) Unused() {{}}\n")).unwrap();
            let symbols = golang::collect(Path::new(&root), &doc, "go").unwrap();
            assert!(symbols.complete, "{:?}", symbols.limitations);
            let required = format!("{receiver}.Read");
            assert!(
                symbols
                    .imports
                    .iter()
                    .any(|symbol| symbol.from == "app/app.go" && symbol.name == required)
            );
            assert!(
                !symbols
                    .imports
                    .iter()
                    .any(|symbol| symbol.name == format!("{receiver}.Unused"))
            );
            archgraph::surface::attach(&mut doc, symbols, 50).unwrap();
            let api = doc
                .surface
                .as_ref()
                .unwrap()
                .modules
                .iter()
                .find(|module| module.module == "api")
                .unwrap();
            assert!(
                !api.could_be_internal
                    .iter()
                    .any(|symbol| symbol.name == required)
            );
            assert!(
                api.could_be_internal
                    .iter()
                    .any(|symbol| symbol.name == format!("{receiver}.Unused"))
            );
        }
    }
}
