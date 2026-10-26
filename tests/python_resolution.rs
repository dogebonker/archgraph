use boundrail::{collect::grimp, model::ImportReason};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn collect(
    name: &str,
    sources: &[(&str, &str)],
    paths: &[&str],
    src: &str,
) -> (PathBuf, boundrail::collect::RawGraph) {
    collect_with_probe(name, sources, paths, src, "")
}

fn collect_with_probe(
    name: &str,
    sources: &[(&str, &str)],
    paths: &[&str],
    src: &str,
    probe: &str,
) -> (PathBuf, boundrail::collect::RawGraph) {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("python-resolution-{name}-{}", std::process::id()));
    let root = directory.join("repo");
    fs::create_dir_all(&root).unwrap();
    for (path, source) in sources {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let stub = r#"
import sys,types
class Graph:
    modules = {'pkg','pkg.app'}
    def find_modules_directly_imported_by(self,module): return []
g = types.ModuleType('grimp')
g.__version__ = 'test'
g.build_graph = lambda *args,**kwargs: Graph()
sys.modules['grimp'] = g
"#;
    let helper = format!(
        "{stub}\n{probe}\n{}",
        include_str!("../src/collect/grimp_dump.py")
    );
    let mut command = Command::new("python3");
    command
        .args(["-I", "-B", "-c", &helper])
        .arg(root.join(src))
        .arg("pkg")
        .arg(&root);
    for path in paths {
        command.arg("--python-path").arg(root.join(path));
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let graph = grimp::parse(&output.stdout, &root, src).unwrap();
    (root, graph)
}

#[test]
fn local_module_probes_require_exact_case_on_every_platform() {
    let (_, graph) = collect_with_probe(
        "case-exact",
        &[
            ("pkg/__init__.py", "from .context import Context\n"),
            (
                "pkg/app.py",
                "from pkg import Context\nimport pkg.Context\nimport pkg.Space.child\nimport pkg.space.child\nimport pkg.Space\nimport pkg.space\nimport pkg.Native\nimport pkg.native\n",
            ),
            ("pkg/context.py", "class Context: pass\n"),
            ("pkg/space/child.py", "value = 1\n"),
            ("pkg/native.so", "not loaded"),
        ],
        &[],
        ".",
        r#"
import os
original_isfile, original_isdir = os.path.isfile, os.path.isdir
def insensitive_path(path):
    current = os.path.sep
    for part in os.path.abspath(path).split(os.path.sep)[1:]:
        try:
            part = next(name for name in os.listdir(current) if name.lower() == part.lower())
        except (OSError, StopIteration):
            return path
        current = os.path.join(current, part)
    return current
os.path.isfile = lambda path: original_isfile(insensitive_path(path))
os.path.isdir = lambda path: original_isdir(insensitive_path(path))
"#,
    );
    let imports = graph.imports.as_ref().unwrap();
    for specifier in ["pkg.Context", "pkg.Space.child", "pkg.Space", "pkg.Native"] {
        let import = imports.iter().find(|i| i.specifier == specifier).unwrap();
        assert!(import.targets.is_empty(), "{specifier}: {import:?}");
        assert!(import.namespace.is_none(), "{specifier}: {import:?}");
        assert!(matches!(import.unresolved, Some(ImportReason::Missing)));
    }
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "pkg" && i.targets == ["pkg/__init__.py"])
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == "pkg/app.py" && edge.to == "pkg/context.py")
    );
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "pkg.space.child" && i.targets == ["pkg/space/child.py"])
    );
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "pkg.space" && i.namespace.as_deref() == Some("pkg/space"))
    );
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "pkg.native"
                && matches!(i.unresolved, Some(ImportReason::Native)))
    );
    assert!(
        graph
            .files
            .iter()
            .all(|file| !file.contains("Context.py") && !file.contains("Space"))
    );
}

#[test]
fn ledger_counts_imports_independently_of_edges_and_lists_failures() {
    let (_, graph) = collect(
        "ledger",
        &[
            ("pkg/__init__.py", ""),
            (
                "pkg/app.py",
                "import json\nimport boundrail_definitely_missing_package\nfrom . import sibling\nfrom .missing import Thing\nimport importlib\nimportlib.import_module(name)\n",
            ),
            ("pkg/sibling.py", "class Thing: pass\n"),
        ],
        &[],
        ".",
    );
    let imports = graph.imports.as_ref().unwrap();
    assert_eq!(imports.len(), 6);
    assert_eq!(
        imports
            .iter()
            .find(|i| i.specifier == "json")
            .unwrap()
            .external
            .as_deref(),
        Some("json")
    );
    assert!(matches!(
        imports
            .iter()
            .find(|i| i.specifier == "boundrail_definitely_missing_package")
            .unwrap()
            .unresolved,
        Some(ImportReason::Missing)
    ));
    assert!(matches!(
        imports
            .iter()
            .find(|i| i.specifier == ".missing")
            .unwrap()
            .unresolved,
        Some(ImportReason::Missing)
    ));
    assert!(matches!(
        imports
            .iter()
            .find(|i| i.specifier == "name")
            .unwrap()
            .unresolved,
        Some(ImportReason::Dynamic)
    ));
    assert_eq!(
        imports.iter().find(|i| i.specifier == ".").unwrap().targets,
        ["pkg/sibling.py"]
    );
}

#[test]
fn src_layout_python_path_and_namespace_resolution_keep_root_relative_targets() {
    let (_, graph) = collect(
        "paths",
        &[
            ("src/pkg/__init__.py", ""),
            (
                "src/pkg/app.py",
                "from extra.ns import Thing\nimport extra\nfrom extra import absent\nimport _native\n",
            ),
            ("lib/extra/ns.py", "class Thing: pass\n"),
            ("src/_native.so", "not loaded"),
        ],
        &["lib"],
        "src",
    );
    let imports = graph.imports.as_ref().unwrap();
    assert_eq!(
        imports
            .iter()
            .find(|i| i.specifier == "extra.ns")
            .unwrap()
            .targets,
        ["lib/extra/ns.py"]
    );
    assert!(graph.files.iter().any(|file| file == "lib/extra/ns.py"));
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "extra" && i.namespace.as_deref() == Some("lib/extra"))
    );
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "extra" && matches!(i.unresolved, Some(ImportReason::Missing)))
    );
    assert!(matches!(
        imports
            .iter()
            .find(|i| i.specifier == "_native")
            .unwrap()
            .unresolved,
        Some(ImportReason::Native)
    ));
}

#[test]
fn external_python_path_package_is_proven_without_running_its_initializer() {
    let (_, graph) = collect(
        "external",
        &[
            ("pkg/__init__.py", ""),
            ("pkg/app.py", "import outpkg.child\n"),
            (
                "../dependency/outpkg/__init__.py",
                "raise RuntimeError('must not execute external source')\n",
            ),
            ("../dependency/outpkg/child.py", "value = 1\n"),
        ],
        &["../dependency"],
        ".",
    );
    let imports = graph.imports.as_ref().unwrap();
    assert_eq!(imports[0].external.as_deref(), Some("outpkg"));
    assert!(imports[0].unresolved.is_none());
    assert!(graph.files.iter().all(|file| !file.contains("dependency")));
}

#[test]
fn namespace_package_roots_and_relative_imports_need_no_initializer() {
    let (root, graph) = collect(
        "namespace",
        &[
            ("pkg/sub/app.py", "from . import sibling\n"),
            ("pkg/sub/sibling.py", "value = 1\n"),
        ],
        &[],
        ".",
    );
    assert_eq!(
        grimp::package(&root, ".", None, &boundrail::collect::globset(&[]).unwrap()).unwrap(),
        "pkg"
    );
    assert!(
        graph
            .imports
            .as_ref()
            .unwrap()
            .iter()
            .any(|i| i.targets == ["pkg/sub/sibling.py"])
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == "pkg/sub/app.py" && edge.to == "pkg/sub/sibling.py")
    );
}

#[test]
fn legacy_recordings_do_not_invent_import_resolution_coverage() {
    let graph = grimp::parse(
        br#"{"grimp_version":"test","modules":[],"imports":[]}"#,
        Path::new("."),
        ".",
    )
    .unwrap();
    assert!(graph.imports.is_none());
}

#[test]
fn literal_and_aliased_dynamic_imports_keep_resolution_evidence() {
    let (_, graph) = collect(
        "dynamic-alias",
        &[
            ("pkg/__init__.py", ""),
            (
                "pkg/app.py",
                "from importlib import import_module as load\nload('pkg.sibling')\ndef deferred():\n    load('pkg.deferred')\n    load(computed)\n",
            ),
            ("pkg/sibling.py", "value = 1\n"),
            ("pkg/deferred.py", "value = 2\n"),
        ],
        &[],
        ".",
    );
    let imports = graph.imports.as_ref().unwrap();
    assert_eq!(
        imports
            .iter()
            .find(|i| i.specifier == "pkg.sibling")
            .unwrap()
            .targets,
        ["pkg/sibling.py"]
    );
    assert!(matches!(
        imports
            .iter()
            .find(|i| i.specifier == "computed")
            .unwrap()
            .unresolved,
        Some(ImportReason::Dynamic)
    ));
    assert!(graph.edges.iter().any(|edge| edge.from == "pkg/app.py"
        && edge.to == "pkg/sibling.py"
        && edge.kind == boundrail::model::EdgeKind::Static));
    assert!(graph.edges.iter().any(|edge| edge.from == "pkg/app.py"
        && edge.to == "pkg/deferred.py"
        && edge.kind == boundrail::model::EdgeKind::Dynamic));
    assert_eq!(
        imports
            .iter()
            .find(|i| i.specifier == "pkg.sibling")
            .unwrap()
            .kind,
        boundrail::model::EdgeKind::Static
    );
    assert_eq!(
        imports
            .iter()
            .find(|i| i.specifier == "pkg.deferred")
            .unwrap()
            .kind,
        boundrail::model::EdgeKind::Dynamic
    );
}

#[test]
fn os_path_is_a_proven_standard_library_alias() {
    let (_, graph) = collect(
        "os-path",
        &[
            ("pkg/__init__.py", ""),
            ("pkg/app.py", "import os.path\nfrom os.path import join\n"),
        ],
        &[],
        ".",
    );
    let imports = graph.imports.as_ref().unwrap();
    assert_eq!(imports.len(), 2);
    assert!(
        imports
            .iter()
            .all(|import| import.external.as_deref() == Some("os") && import.unresolved.is_none())
    );
}

#[test]
fn ordinary_or_shadowed_import_module_calls_do_not_invent_imports() {
    for (name, source, expected_imports) in [
        ("unbound", "importlib.import_module('pkg.sibling')\n", 0),
        (
            "object",
            "class Loader:\n    def import_module(self, name): return name\nimportlib = Loader()\nimportlib.import_module('pkg.sibling')\n",
            0,
        ),
        (
            "parameter",
            "import importlib as il\ndef load(il):\n    il.import_module('pkg.sibling')\n",
            1,
        ),
        (
            "reassigned",
            "from importlib import import_module as load\nload = lambda name: name\nload('pkg.sibling')\n",
            1,
        ),
        (
            "builtin",
            "def __import__(name): return name\n__import__('pkg.sibling')\n",
            0,
        ),
        (
            "monkeypatched",
            "import importlib\nimportlib.import_module = lambda name: name\nimportlib.import_module('pkg.sibling')\n",
            1,
        ),
        (
            "default-binding",
            "import importlib\ndef replace(value=(importlib := None)): pass\nimportlib.import_module('pkg.sibling')\n",
            1,
        ),
        (
            "global",
            "import importlib as il\ndef replace():\n    global il\n    il = None\ndef load():\n    il.import_module('pkg.sibling')\n",
            1,
        ),
    ] {
        let (_, graph) = collect(
            name,
            &[
                ("pkg/__init__.py", ""),
                ("pkg/app.py", source),
                ("pkg/sibling.py", "value = 1\n"),
            ],
            &[],
            ".",
        );
        let imports = graph.imports.as_ref().unwrap();
        assert_eq!(imports.len(), expected_imports, "{name}: {imports:?}");
        assert!(
            !graph
                .edges
                .iter()
                .any(|edge| edge.from == "pkg/app.py" && edge.to == "pkg/sibling.py"),
            "{name}"
        );
    }
}

#[test]
fn proven_importlib_and_builtin_bindings_work_at_module_and_function_scope() {
    let (_, graph) = collect(
        "proven-bindings",
        &[
            ("pkg/__init__.py", ""),
            (
                "pkg/app.py",
                "import importlib; importlib.import_module('pkg.top')\ndef deferred():\n    import importlib as il\n    il.import_module('pkg.later')\n    __import__('pkg.builtin')\n",
            ),
            ("pkg/top.py", "value = 1\n"),
            ("pkg/later.py", "value = 2\n"),
            ("pkg/builtin.py", "value = 3\n"),
        ],
        &[],
        ".",
    );
    for (target, kind) in [
        ("pkg/top.py", boundrail::model::EdgeKind::Static),
        ("pkg/later.py", boundrail::model::EdgeKind::Dynamic),
        ("pkg/builtin.py", boundrail::model::EdgeKind::Dynamic),
    ] {
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.from == "pkg/app.py" && edge.to == target && edge.kind == kind),
            "{target}: {:?}",
            graph.edges
        );
    }
}
