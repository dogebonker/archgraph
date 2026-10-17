use std::fs;
use std::path::PathBuf;
use std::process::Command;

use archgraph::collect::grimp;
use archgraph::model::EdgeKind;

fn fixture(name: &str, sources: &[(&str, &str)]) -> (PathBuf, Vec<u8>) {
    fixture_options(name, sources, false)
}

fn fixture_options(
    name: &str,
    sources: &[(&str, &str)],
    include_tests: bool,
) -> (PathBuf, Vec<u8>) {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("python-barrels-{name}"));
    fs::create_dir_all(&root).unwrap();
    let mut modules = Vec::new();
    for (path, source) in sources {
        let file = root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, source).unwrap();
        let module = path
            .strip_prefix("src/")
            .unwrap_or(path)
            .strip_suffix("/__init__.py")
            .unwrap_or_else(|| {
                path.strip_prefix("src/")
                    .unwrap_or(path)
                    .strip_suffix(".py")
                    .unwrap()
            });
        modules.push(module.replace('/', "."));
    }
    modules.retain(|module| module == "pkg" || module.starts_with("pkg."));
    // Stub only grimp's graph transport, so the production AST pass runs without a Python
    // dependency or importing the fixture's source code.
    let stub = format!(
        r#"
import sys, types
class Graph:
    modules = {modules}
    def find_modules_directly_imported_by(self, module): return []
g = types.ModuleType('grimp')
g.__version__ = 'test'
g.build_graph = lambda *args, **kwargs: Graph()
sys.modules['grimp'] = g
"#,
        modules = serde_json::to_string(&modules).unwrap()
    );
    let helper = format!("{stub}\n{}", include_str!("../src/collect/grimp_dump.py"));
    let source_dir = if sources.iter().any(|(path, _)| path.starts_with("src/pkg/")) {
        "src"
    } else {
        "."
    };
    let output = Command::new("python3")
        .args(["-I", "-B", "-c", &helper])
        .arg(root.join(source_dir))
        .arg("pkg")
        .arg(&root)
        .args(include_tests.then_some("--include-tests"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (root, output.stdout)
}

#[test]
fn aliases_and_static_all_follow_a_three_package_star_chain() {
    let (root, dump) = fixture(
        "chain",
        &[
            (
                "src/pkg/__init__.py",
                "from .api import Name as Public\n__all__ = ['Public']\n",
            ),
            (
                "src/pkg/api/__init__.py",
                "from ..middle import *\n__all__ = ['Name']\n",
            ),
            (
                "src/pkg/middle/__init__.py",
                "from ..impl import Thing as Name\n__all__ = ['Name']\n",
            ),
            ("src/pkg/impl.py", "class Thing: pass\n"),
            ("src/pkg/consumer.py", "from pkg import Public\n"),
            ("src/pkg/star_consumer.py", "from pkg import *\n"),
        ],
    );
    let graph = grimp::parse(&dump, &root, "src").unwrap();
    assert!(graph.metadata["src/pkg/__init__.py"].barrel);
    assert!(graph.metadata["src/pkg/api/__init__.py"].barrel);
    assert!(graph.metadata["src/pkg/middle/__init__.py"].barrel);
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == "src/pkg/consumer.py"
                && edge.to == "src/pkg/impl.py"
                && edge.kind == EdgeKind::Static)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == "src/pkg/star_consumer.py"
                && edge.to == "src/pkg/impl.py"
                && edge.kind == EdgeKind::Static)
    );
}

#[test]
fn initialization_code_keeps_its_load_dependency_and_never_executes() {
    let (root, dump) = fixture(
        "mixed",
        &[
            (
                "pkg/__init__.py",
                "from .impl import Thing\nraise RuntimeError('must never execute source')\n",
            ),
            ("pkg/impl.py", "class Thing: pass\nfrom . import consumer\n"),
            ("pkg/consumer.py", "from pkg import Thing\n"),
        ],
    );
    let graph = grimp::parse(&dump, &root, ".").unwrap();
    assert!(
        !graph
            .metadata
            .get("pkg/__init__.py")
            .is_some_and(|node| node.barrel)
    );
    assert!(graph.edges.iter().any(|edge| edge.from == "pkg/consumer.py"
        && edge.to == "pkg/__init__.py"
        && edge.kind == EdgeKind::Static));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == "pkg/impl.py" && edge.to == "pkg/consumer.py")
    );
}

#[test]
fn type_checking_and_function_local_imports_keep_their_kinds_after_resolution() {
    let (root, dump) = fixture(
        "deferred",
        &[
            ("pkg/__init__.py", "from .impl import Thing\n"),
            ("pkg/impl.py", "class Thing: pass\n"),
            (
                "pkg/type_consumer.py",
                "from typing import TYPE_CHECKING as CHECK\nif CHECK:\n    from pkg import Thing\n",
            ),
            (
                "pkg/lazy_consumer.py",
                "def load():\n    from pkg import Thing\n",
            ),
        ],
    );
    let graph = grimp::parse(&dump, &root, ".").unwrap();
    for (file, kind) in [
        ("pkg/type_consumer.py", EdgeKind::Type),
        ("pkg/lazy_consumer.py", EdgeKind::Dynamic),
    ] {
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.from == file && edge.to == "pkg/impl.py" && edge.kind == kind)
        );
    }
}

#[test]
fn old_recordings_without_ast_routes_retain_their_edges() {
    let (root, _) = fixture(
        "legacy",
        &[
            ("pkg/__init__.py", ""),
            ("pkg/impl.py", "class Thing: pass\n"),
        ],
    );
    let dump = br#"{"grimp_version":"test","modules":["pkg","pkg.impl"],"imports":[{"importer":"pkg","imported":"pkg.impl","kind":"type"}]}"#;
    let graph = grimp::parse(dump, &root, ".").unwrap();
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].kind, EdgeKind::Type);
}

#[test]
fn declared_package_attribute_precedes_a_same_named_submodule() {
    let (root, dump) = fixture(
        "attribute",
        &[
            (
                "pkg/__init__.py",
                "from .impl import Thing as child\n__all__ = ['child']\n",
            ),
            ("pkg/impl.py", "class Thing: pass\n"),
            ("pkg/child.py", "class Other: pass\n"),
            ("pkg/consumer.py", "from pkg import child\n"),
        ],
    );
    let graph = grimp::parse(&dump, &root, ".").unwrap();
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == "pkg/consumer.py" && edge.to == "pkg/impl.py")
    );
    assert!(
        !graph
            .edges
            .iter()
            .any(|edge| edge.from == "pkg/consumer.py" && edge.to == "pkg/child.py")
    );
}

#[test]
fn star_bound_package_attribute_precedes_a_same_named_submodule() {
    let (root, dump) = fixture(
        "star-attribute",
        &[
            ("pkg/__init__.py", "from .impl import *\n"),
            ("pkg/impl.py", "class Thing: pass\n"),
            ("pkg/Thing.py", "class Other: pass\n"),
            ("pkg/consumer.py", "from pkg import Thing\n"),
        ],
    );
    let ast: serde_json::Value = serde_json::from_slice(&dump).unwrap();
    let consumer = ast["barrel_modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["file"] == "pkg/consumer.py")
        .unwrap();
    assert_eq!(consumer["imports"][0]["to"], "pkg/__init__.py");
    let graph = grimp::parse(&dump, &root, ".").unwrap();
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == "pkg/consumer.py" && edge.to == "pkg/impl.py")
    );
    assert!(
        !graph
            .edges
            .iter()
            .any(|edge| edge.from == "pkg/consumer.py" && edge.to == "pkg/Thing.py")
    );
}

#[test]
fn package_relative_submodule_import_does_not_bind_itself() {
    let (root, dump) = fixture(
        "self-binding",
        &[
            (
                "pkg/__init__.py",
                "from . import handlers, schemas\n__all__ = ['handlers', 'schemas']\n",
            ),
            ("pkg/handlers.py", "def handle(): pass\n"),
            ("pkg/schemas.py", "class Schema: pass\n"),
        ],
    );
    let graph = grimp::parse(&dump, &root, ".").unwrap();
    for destination in ["pkg/handlers.py", "pkg/schemas.py"] {
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.from == "pkg/__init__.py" && edge.to == destination)
        );
    }
    assert!(!graph.edges.iter().any(|edge| edge.from == edge.to));
    assert!(!graph.edges.iter().any(|edge| {
        edge.resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("cycle"))
    }));
}

#[test]
fn included_tests_outside_a_src_package_are_discovered_without_execution() {
    let sources = [
        ("src/pkg/__init__.py", "from .impl import Thing\n"),
        ("src/pkg/impl.py", "class Thing: pass\n"),
        (
            "tests/test_feature.py",
            "from pkg import Thing\nraise RuntimeError('must not execute tests')\n",
        ),
        ("tests/support.py", "from pkg.impl import Thing\n"),
    ];
    let (root, dump) = fixture_options("external-tests", &sources, true);
    let graph = grimp::parse(&dump, &root, "src").unwrap();
    assert!(
        graph
            .files
            .iter()
            .any(|file| file == "tests/test_feature.py")
    );
    assert!(graph.files.iter().any(|file| file == "tests/support.py"));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == "tests/test_feature.py" && edge.to == "src/pkg/impl.py")
    );
    let (root, dump) = fixture_options("external-tests-disabled", &sources, false);
    let graph = grimp::parse(&dump, &root, "src").unwrap();
    assert!(!graph.files.iter().any(|file| file.starts_with("tests/")));
}

#[test]
fn included_external_tests_cover_modules_without_creating_cycles() {
    let (root, dump) = fixture_options(
        "test-coverage",
        &[
            ("src/pkg/__init__.py", ""),
            (
                "src/pkg/impl.py",
                "class Thing: pass\nfrom tests.test_feature import test_feature\n",
            ),
            (
                "tests/test_feature.py",
                "from pkg.impl import Thing\ndef test_feature(): pass\n",
            ),
        ],
        true,
    );
    let recording = root.join("grimp.json");
    fs::write(&recording, dump).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .arg("scan")
        .arg(&root)
        .args([
            "--language",
            "python",
            "--package",
            "pkg",
            "--include-tests",
            "--collector-input",
        ])
        .arg(&recording)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let test = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "tests/test_feature.py")
        .unwrap();
    assert_eq!(test["class"], "tests");
    assert!(doc["cycles"].as_array().unwrap().is_empty());
    assert!(doc["test_coverage"]["covered"].as_u64().unwrap() > 0);
    assert!(
        !doc["test_coverage"]["without_importing_tests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|module| module == "impl.py")
    );
}
