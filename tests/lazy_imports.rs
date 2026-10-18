use archgraph::collect::depcruise;
use archgraph::model::EdgeKind;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Source(PathBuf);

impl Source {
    fn new(source: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("archgraph-lazy-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("entry.ts"), source).unwrap();
        Self(path)
    }

    fn parse(&self, dependencies: Value) -> Vec<archgraph::model::Edge> {
        let input = json!({
            "modules": [{"source": "entry.ts", "dependencies": dependencies}, {"source": "target.ts"}],
            "summary": {"totalCruised": 2}
        });
        depcruise::parse_at(
            &serde_json::to_vec(&input).unwrap(),
            &mut archgraph::collect::imports::Sources::new(&self.0).unwrap(),
        )
        .unwrap()
        .edges
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn dependency(specifier: &str, types: &[&str]) -> Value {
    json!({"module": specifier, "resolved": "target.ts", "dependencyTypes": types})
}

#[test]
fn source_scope_refines_only_function_requires() {
    for (source, expected) in [
        ("const target = require('./target');", EdgeKind::Static),
        (
            "function load() { return require('./target'); }",
            EdgeKind::Dynamic,
        ),
        ("const load = () => require('./target');", EdgeKind::Dynamic),
        ("if (enabled) { require('./target'); }", EdgeKind::Static),
    ] {
        let source = Source::new(source);
        let edges = source.parse(json!([dependency("./target", &["local", "require"])]));
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, expected);
    }
}

#[test]
fn any_top_level_occurrence_dominates_lazy_including_aliases() {
    for (source, dependencies) in [
        (
            "require('./target'); function load() { require('./target'); }",
            json!([dependency("./target", &["require"])]),
        ),
        (
            "require('@target'); function load() { require('./target'); }",
            json!([
                dependency("@target", &["require"]),
                dependency("./target", &["require"])
            ]),
        ),
    ] {
        let source = Source::new(source);
        assert_eq!(source.parse(dependencies)[0].kind, EdgeKind::Static);
    }
}

#[test]
fn compiler_type_and_reexport_metadata_win_over_syntax() {
    let source = Source::new("function load() { require('./target'); }");
    for (types, expected) in [
        (vec!["pre-compilation-only"], EdgeKind::Type),
        (vec!["export"], EdgeKind::Reexport),
        (vec!["export", "type-only"], EdgeKind::TypeReexport),
    ] {
        assert_eq!(
            source.parse(json!([dependency("./target", &types)]))[0].kind,
            expected
        );
    }
}

#[test]
fn missing_source_or_specifier_preserves_collector_evidence() {
    let source = Source::new("function load() { require('./target'); }");
    assert_eq!(
        source.parse(json!([{"resolved": "target.ts"}]))[0].kind,
        EdgeKind::Static
    );
    assert_eq!(
        source.parse(json!([
            dependency("./target", &["require"]),
            {"resolved": "target.ts"}
        ]))[0]
            .kind,
        EdgeKind::Static
    );
    fs::remove_file(source.0.join("entry.ts")).unwrap();
    assert_eq!(
        source.parse(json!([dependency("./target", &["require"])]))[0].kind,
        EdgeKind::Static
    );
}

#[test]
fn tokenizer_candidates_do_not_invent_graph_dependencies() {
    let source = Source::new(
        "require('./target'); import(computed); function load() { require('./phantom'); }",
    );
    assert!(source.parse(json!([])).is_empty());
    let edges = source.parse(json!([dependency("./target", &["require"])]));
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to, "target.ts");
    assert_eq!(edges[0].kind, EdgeKind::Static);
}
