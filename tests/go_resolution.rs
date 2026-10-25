use boundrail::collect::go;
use std::{fs, path::Path};
#[test]
fn old_go_recordings_do_not_fabricate_import_coverage() {
    let mut legacy: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/go-resolve/go.json")).unwrap();
    legacy.as_object_mut().unwrap().remove("imports");
    let parsed = go::parse(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(parsed.raw.imports.is_none());
}
#[test]
#[ignore = "requires Go 1.21 or later"]
fn go_import_ledger_separates_packages_from_symbol_references() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-import-ledger");
    fs::create_dir_all(root.join("local")).unwrap();
    fs::write(
        root.join("go.mod"),
        "module example.com/ledger\n\ngo 1.21\nrequire example.com/dep v1.2.3\n",
    )
    .unwrap();
    fs::write(root.join("app.go"),"package app\nimport (\n \"fmt\"\n \"example.com/ledger/local\"\n _ \"example.com/dep/pkg\"\n _ \"example.com/missing\"\n)\nfunc Uses() { fmt.Println(local.Thing,local.Thing) }\n").unwrap();
    fs::write(
        root.join("local/local.go"),
        "package local\nconst Thing = 1\nfunc init(){panic(\"must not execute\")}\n",
    )
    .unwrap();
    let dump = go::run(&root, "go", &[".".into()]).unwrap();
    let parsed = go::parse(&dump).unwrap();
    let records = parsed.raw.imports.unwrap();
    assert_eq!(records.len(), 4);
    assert_eq!(
        records
            .iter()
            .find(|r| r.specifier == "fmt")
            .unwrap()
            .external
            .as_deref(),
        Some("fmt")
    );
    assert_eq!(
        records
            .iter()
            .find(|r| r.specifier == "example.com/ledger/local")
            .unwrap()
            .targets,
        ["local/local.go"]
    );
    assert_eq!(
        records
            .iter()
            .find(|r| r.specifier == "example.com/dep/pkg")
            .unwrap()
            .external
            .as_deref(),
        Some("example.com/dep/pkg")
    );
    assert!(
        records
            .iter()
            .find(|r| r.specifier == "example.com/missing")
            .unwrap()
            .unresolved
            .is_some()
    );
    assert!(records.iter().all(|r| r.line.is_some_and(|line| line > 0)));
}
