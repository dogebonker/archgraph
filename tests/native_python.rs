use std::fs;
use std::path::PathBuf;

use boundrail::collect::excludes::{Options, Policy};
use boundrail::collect::native::python;
use boundrail::model::{EdgeKind, ImportReason, Language};

struct Fixture(PathBuf);
impl Fixture {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture(files: &[(&str, &str)]) -> Fixture {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = Fixture(std::env::temp_dir().join(format!(
        "archgraph-native-python-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )));
    fs::create_dir_all(dir.path()).unwrap();
    for (path, contents) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    dir
}
fn collect(
    dir: &Fixture,
    package: Option<&str>,
    paths: &[PathBuf],
) -> boundrail::collect::native::Output {
    let policy = Policy::new(dir.path(), Language::Python, Options::default()).unwrap();
    python::collect(dir.path(), "src", package, paths, &policy).unwrap()
}
#[test]
fn native_python_relative_src_namespace_and_exclusions() {
    let dir = fixture(&[
        (
            "src/pkg/a.py",
            "from .b import Thing\nfrom pkg.ns.c import C\nfrom .test_hidden import Hidden\n",
        ),
        ("src/pkg/b.py", "class Thing: pass\n"),
        ("src/pkg/test_hidden.py", "class Hidden: pass\n"),
        ("src/pkg/ns/c.py", "class C: pass\n"),
    ]);
    let output = collect(&dir, None, &[]);
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "src/pkg/a.py" && e.to == "src/pkg/b.py")
    );
    assert!(output.raw.edges.iter().any(|e| e.to == "src/pkg/ns/c.py"));
    assert!(output.raw.files.contains(&"src/pkg/test_hidden.py".into()));
    assert!(
        !output
            .raw
            .edges
            .iter()
            .any(|e| e.to == "src/pkg/test_hidden.py")
    );
    let record = output
        .raw
        .imports
        .unwrap()
        .into_iter()
        .find(|i| i.specifier == ".test_hidden")
        .unwrap();
    assert_eq!(record.targets, ["src/pkg/test_hidden.py"]);
    assert!(record.unresolved.is_none());
}
#[test]
fn native_python_attributes_precede_submodules_and_barrels_forward_symbols() {
    let dir = fixture(&[
        (
            "src/pkg/__init__.py",
            "from .model import Thing as Thing\n__all__ = ['Thing']\n",
        ),
        ("src/pkg/model.py", "class Thing: pass\n"),
        ("src/pkg/Thing.py", "wrong = True\n"),
        ("src/pkg/consumer.py", "from pkg import Thing\n"),
    ]);
    let output = collect(&dir, None, &[]);
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "src/pkg/consumer.py" && e.to == "src/pkg/model.py")
    );
    assert!(
        !output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "src/pkg/consumer.py" && e.to == "src/pkg/Thing.py")
    );
    assert!(output.raw.metadata["src/pkg/__init__.py"].barrel);
    assert!(
        output
            .symbols
            .exports
            .iter()
            .any(|s| s.file == "src/pkg/__init__.py"
                && s.name == "Thing"
                && s.target.file == "src/pkg/model.py")
    );
}
#[test]
fn native_python_type_lazy_dynamic_and_missing_are_accounted() {
    let dir = fixture(&[
        (
            "src/pkg/a.py",
            "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    from .b import B\nelse:\n    from .c import C\ndef f():\n    from .b import B\nimport importlib\nimportlib.import_module(name)\nimport definitely_not_installed\nimport configparser, tomllib\n",
        ),
        ("src/pkg/b.py", "class B: pass\n"),
        ("src/pkg/c.py", "class C: pass\n"),
    ]);
    let records = collect(&dir, Some("pkg"), &[]).raw.imports.unwrap();
    assert!(
        records
            .iter()
            .any(|i| i.kind == EdgeKind::Type && i.specifier == ".b")
    );
    assert!(
        records
            .iter()
            .any(|i| i.kind == EdgeKind::Dynamic && i.specifier == ".b")
    );
    assert!(
        records
            .iter()
            .any(|i| matches!(i.unresolved, Some(ImportReason::Dynamic)))
    );
    assert!(
        records
            .iter()
            .any(|i| i.specifier == "definitely_not_installed"
                && matches!(i.unresolved, Some(ImportReason::Missing)))
    );
    assert!(
        records
            .iter()
            .any(|i| i.external.as_deref() == Some("typing"))
    );
    assert!(
        records
            .iter()
            .any(|i| i.specifier == ".c" && i.kind == EdgeKind::Static)
    );
    for module in ["configparser", "tomllib"] {
        assert!(
            records
                .iter()
                .any(|i| i.external.as_deref() == Some(module))
        );
    }
}
#[test]
fn native_python_policy_decides_build_named_subpackages() {
    let dir = fixture(&[
        ("src/pkg/a.py", "from .target import helper\n"),
        ("src/pkg/target/helper.py", "value = 1\n"),
    ]);
    let output = collect(&dir, Some("pkg"), &[]);
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "src/pkg/a.py" && e.to == "src/pkg/target/helper.py")
    );
}
#[test]
fn native_python_repeatable_paths_and_no_project_execution() {
    let dir = fixture(&[
        ("src/pkg/a.py", "from other.b import B\n"),
        ("lib/other/b.py", "class B: pass\n"),
        (
            "src/pkg/__init__.py",
            "raise RuntimeError('must not execute')\n",
        ),
        (
            "sitecustomize.py",
            "raise RuntimeError('must not execute')\n",
        ),
    ]);
    let output = collect(&dir, Some("pkg"), &["lib".into()]);
    assert!(output.raw.edges.iter().any(|e| e.to == "lib/other/b.py"));
    let policy = Policy::new(dir.path(), Language::Python, Options::default()).unwrap();
    assert!(python::collect(dir.path(), "src", None, &["missing".into()], &policy).is_err());
}
#[test]
fn native_python_star_forwarding_stays_conservative_and_keeps_syntax_errors() {
    let dir = fixture(&[
        ("src/pkg/__init__.py", "from .model import *\n"),
        ("src/pkg/model.py", "class Thing: pass\n"),
        ("src/pkg/a.py", "from pkg import Thing\n"),
        ("src/pkg/b.py", "import os\n\ndef broken(:\n"),
    ]);
    let output = collect(&dir, None, &[]);
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "src/pkg/a.py" && e.to == "src/pkg/model.py")
    );
    assert!(
        output
            .raw
            .imports
            .unwrap()
            .iter()
            .any(|i| i.from == "src/pkg/b.py" && i.external.as_deref() == Some("os"))
    );
    assert!(!output.symbols.complete);
}
#[test]
fn native_python_installed_metadata_proves_import_names_not_distribution_names() {
    let dir = fixture(&[
        ("src/pkg/a.py", "import PIL\nimport Pillow\n"),
        (
            ".venv/lib/python3.14/site-packages/Pillow-1.dist-info/top_level.txt",
            "PIL\n",
        ),
    ]);
    let records = collect(&dir, None, &[]).raw.imports.unwrap();
    assert!(
        records
            .iter()
            .any(|i| i.specifier == "PIL" && i.external.as_deref() == Some("PIL"))
    );
    assert!(records.iter().any(|i| i.specifier == "Pillow" && matches!(i.unresolved, Some(ImportReason::Missing))));
}
#[test]
fn native_python_self_package_import_uses_only_prior_bindings() {
    let dir = fixture(&[
        (
            "src/pkg/__init__.py",
            "from . import Thing\nclass Thing: pass\n",
        ),
        ("src/pkg/Thing.py", "value = 1\n"),
    ]);
    let output = collect(&dir, None, &[]);
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "src/pkg/__init__.py" && e.to == "src/pkg/Thing.py")
    );
}
#[test]
fn native_python_missing_namespace_member_is_not_resolved_by_parent_alone() {
    let dir = fixture(&[
        ("src/pkg/ns/a.py", "value = 1\n"),
        ("src/pkg/main.py", "from pkg.ns import missing\n"),
    ]);
    let records = collect(&dir, None, &[]).raw.imports.unwrap();
    assert!(
        records
            .iter()
            .any(|i| matches!(i.unresolved, Some(ImportReason::Missing)))
    );
}
#[test]
fn native_python_import_call_aliases_respect_lexical_shadowing() {
    let dir = fixture(&[
        (
            "src/pkg/a.py",
            "import importlib as il\nfrom importlib import import_module as load\nil.import_module('pkg.b')\nload('pkg.b')\ndef f(load):\n    load('pkg.missing')\ndef g(il):\n    il.import_module('pkg.missing')\ndef h(__import__):\n    __import__('pkg.missing')\n",
        ),
        ("src/pkg/b.py", "value = 1\n"),
    ]);
    let records = collect(&dir, None, &[]).raw.imports.unwrap();
    assert_eq!(records.iter().filter(|i| i.specifier == "pkg.b").count(), 2);
    assert!(!records.iter().any(|i| i.specifier == "pkg.missing"));
}
#[test]
fn native_python_follows_imports_outside_package_without_scanning_unrelated_sources() {
    let dir = fixture(&[
        ("src/pkg/a.py", "from other.b import B\n"),
        ("src/other/b.py", "from third.c import C\nclass B: pass\n"),
        ("src/third/c.py", "class C: pass\n"),
        ("src/unrelated/bad.py", "import bogus_unrelated\n"),
    ]);
    let output = collect(&dir, Some("pkg"), &[]);
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "src/other/b.py" && e.to == "src/third/c.py")
    );
    assert!(!output.raw.files.iter().any(|p| p.contains("unrelated")));
}
#[test]
fn native_python_function_import_does_not_bind_package_attribute() {
    let dir = fixture(&[
        (
            "src/pkg/__init__.py",
            "def setup():\n    from . import exc\n",
        ),
        ("src/pkg/exc.py", "class Error: pass\n"),
        ("src/pkg/consumer.py", "from pkg import exc\n"),
    ]);
    let output = collect(&dir, Some("pkg"), &[]);
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "src/pkg/consumer.py" && e.to == "src/pkg/exc.py")
    );
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "src/pkg/__init__.py"
                && e.to == "src/pkg/exc.py"
                && e.kind == EdgeKind::Dynamic)
    );
}
#[test]
fn native_python_typing_alias_guards_future_imports_and_plain_lazy_imports() {
    let dir = fixture(&[
        (
            "src/pkg/a.py",
            "from __future__ import annotations\nimport typing as t\nfrom typing import TYPE_CHECKING as TC\nif t.TYPE_CHECKING:\n    from .b import B\nif TC:\n    from .c import C\ndef load():\n    import pkg.c\n",
        ),
        ("src/pkg/b.py", "class B: pass\n"),
        ("src/pkg/c.py", "class C: pass\n"),
    ]);
    let output = collect(&dir, Some("pkg"), &[]);
    assert!(
        output
            .raw
            .imports
            .as_ref()
            .unwrap()
            .iter()
            .any(|i| i.external.as_deref() == Some("__future__"))
    );
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.to == "src/pkg/b.py" && e.kind == EdgeKind::Type)
    );
    let records = output.raw.imports.unwrap();
    assert!(
        records
            .iter()
            .any(|i| i.specifier == ".c" && i.kind == EdgeKind::Type)
    );
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.to == "src/pkg/c.py" && e.kind == EdgeKind::Dynamic)
    );
}
#[test]
fn native_python_reference_scope_discovers_production_dependencies_from_excluded_tests() {
    let dir = fixture(&[
        ("src/pkg/a.py", "value=1\n"),
        ("src/pkg/test_scope.py", "from extra.b import B\n"),
        ("src/extra/b.py", "from another.c import C\nclass B: pass\n"),
        ("src/another/c.py", "class C: pass\n"),
        ("src/unrelated/d.py", "value=1\n"),
    ]);
    fs::write(dir.path().join("src/pkg/test_unreadable.py"), [0xff]).unwrap();
    let output = collect(&dir, Some("pkg"), &[]);
    assert!(
        output
            .symbols
            .limitations
            .iter()
            .any(|s| s.contains("excluded source unavailable"))
    );
    assert!(output.raw.files.contains(&"src/pkg/test_scope.py".into()));
    assert!(
        !output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "src/pkg/test_scope.py")
    );
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "src/extra/b.py" && e.to == "src/another/c.py")
    );
    assert!(!output.raw.files.contains(&"src/unrelated/d.py".into()));
}
#[test]
fn native_python_barrel_all_filters_surface_without_inventing_wildcard_target_exports() {
    let dir = fixture(&[
        (
            "src/pkg/__init__.py",
            "from .one import *\nfrom .one import Thing\nfrom .two import *\n__all__=['Thing']\n",
        ),
        ("src/pkg/one.py", "__all__=['Thing']\nclass Thing: pass\n"),
        (
            "src/pkg/two.py",
            "from .one import Thing\n__all__=['Other']\nclass Other: pass\n",
        ),
        ("src/consumer/a.py", "from pkg import Thing\n"),
    ]);
    let output = collect(&dir, Some("consumer"), &[]);
    let edges: Vec<_> = output
        .raw
        .edges
        .iter()
        .filter(|e| e.from == "src/consumer/a.py")
        .collect();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to, "src/pkg/one.py");
    assert_eq!(
        edges[0].confidence,
        Some(boundrail::model::Confidence::High)
    );
}
#[test]
fn native_python_distinct_wildcard_providers_remain_ambiguous() {
    let dir = fixture(&[
        (
            "src/pkg/__init__.py",
            "from .one import *\nfrom .two import *\n__all__=['Thing']\n",
        ),
        ("src/pkg/one.py", "__all__=['Thing']\nclass Thing: pass\n"),
        ("src/pkg/two.py", "__all__=['Thing']\nclass Thing: pass\n"),
        ("src/consumer/a.py", "from pkg import Thing\n"),
    ]);
    let output = collect(&dir, Some("consumer"), &[]);
    let edge = output
        .raw
        .edges
        .iter()
        .find(|e| e.from == "src/consumer/a.py")
        .unwrap();
    assert_eq!(edge.to, "src/pkg/__init__.py");
    assert_eq!(edge.confidence, Some(boundrail::model::Confidence::Low));
    assert!(
        edge.resolution_reason
            .as_ref()
            .unwrap()
            .contains("ambiguous star export")
    );
}

#[test]
fn native_python_wheel_record_proves_installed_import_roots() {
    let dir = fixture(&[
        (
            "src/pkg/a.py",
            "import fastapi\nimport uvicorn\nimport custom_module\nimport google.cloud\nimport speed\nimport invented_distribution\n",
        ),
        (
            ".venv/lib/python3.14/site-packages/invented_distribution-1.dist-info/RECORD",
            "fastapi/__init__.py,,\nuvicorn/main.py,,\ncustom_module.py,,\ngoogle/cloud/client.py,,\nspeed.cpython-314-darwin.so,,\n",
        ),
        (
            ".venv/lib/python3.14/site-packages/fastapi/__init__.py",
            "raise RuntimeError('must not execute')\n",
        ),
        (".venv/lib/python3.14/site-packages/uvicorn/main.py", ""),
        (".venv/lib/python3.14/site-packages/custom_module.py", ""),
        (
            ".venv/lib/python3.14/site-packages/google/cloud/client.py",
            "",
        ),
        (
            ".venv/lib/python3.14/site-packages/speed.cpython-314-darwin.so",
            "not executable",
        ),
    ]);
    let records = collect(&dir, None, &[]).raw.imports.unwrap();
    for name in [
        "fastapi",
        "uvicorn",
        "custom_module",
        "google.cloud",
        "speed",
    ] {
        let record = records.iter().find(|i| i.specifier == name).unwrap();
        assert_eq!(
            record.external.as_deref(),
            Some(name.split('.').next().unwrap()),
            "{name}"
        );
        assert!(record.unresolved.is_none(), "{name}");
    }
    assert!(
        records
            .iter()
            .any(|i| i.specifier == "invented_distribution"
                && matches!(i.unresolved, Some(ImportReason::Missing)))
    );
}

#[test]
fn native_python_wheel_records_reject_nonimports_and_missing_files() {
    let dir = fixture(&[
        (
            "src/pkg/a.py",
            "import missing\nimport traversal\nimport payload\nimport stubs\nimport unknown\nimport legitimate\nimport native_mod\n",
        ),
        ("outside.py", ""),
        (
            ".venv/lib/python3.14/site-packages/unrelated-1.dist-info/RECORD",
            "missing/__init__.py,,\n../traversal.py,,\ntraversal/../actual.py,,\n/absolute.py,,\npayload.data/scripts/payload.py,,\nstubs/__init__.pyi,,\nunknown/assets.json,,\n\"legitimate/__init__.py\",\"sha256=a,b\",0\r\nnative_mod.abi3.pyd,,\n",
        ),
        (".venv/lib/python3.14/site-packages/traversal.py", ""),
        (".venv/lib/python3.14/site-packages/actual.py", ""),
        (
            ".venv/lib/python3.14/site-packages/payload.data/scripts/payload.py",
            "",
        ),
        (".venv/lib/python3.14/site-packages/stubs/__init__.pyi", ""),
        (".venv/lib/python3.14/site-packages/unknown/assets.json", ""),
        (
            ".venv/lib/python3.14/site-packages/legitimate/__init__.py",
            "",
        ),
        (".venv/lib/python3.14/site-packages/native_mod.abi3.pyd", ""),
    ]);
    let records = collect(&dir, None, &[]).raw.imports.unwrap();
    for name in ["missing", "traversal", "payload", "stubs", "unknown"] {
        let record = records.iter().find(|i| i.specifier == name).unwrap();
        assert!(
            matches!(record.unresolved, Some(ImportReason::Missing)),
            "{name}"
        );
    }
    for name in ["legitimate", "native_mod"] {
        assert!(
            records
                .iter()
                .any(|i| i.specifier == name && i.external.as_deref() == Some(name))
        );
    }
}

#[test]
fn native_python_wheel_record_works_in_contained_python_path() {
    let dir = fixture(&[
        ("src/pkg/a.py", "import installed\n"),
        (
            "deps/distribution-1.dist-info/RECORD",
            "installed/__init__.py,,\n",
        ),
        (
            "deps/installed/__init__.py",
            "raise RuntimeError('must not execute')\n",
        ),
    ]);
    let records = collect(&dir, None, &["deps".into()]).raw.imports.unwrap();
    // Explicit source paths keep their existing internal-file resolution priority.
    assert!(
        records
            .iter()
            .any(|i| i.specifier == "installed" && i.unresolved.is_none())
    );
    assert!(
        collect(&dir, None, &[])
            .raw
            .imports
            .unwrap()
            .iter()
            .any(|i| i.specifier == "installed"
                && matches!(i.unresolved, Some(ImportReason::Missing)))
    );
}

#[cfg(unix)]
#[test]
fn native_python_wheel_records_do_not_follow_escaping_symlinks() {
    use std::os::unix::fs::symlink;
    let outside = fixture(&[
        ("escaped.py", ""),
        ("stolen/__init__.py", ""),
        ("metadata/RECORD", "forged.py,,\n"),
    ]);
    let dir = fixture(&[
        (
            "src/pkg/a.py",
            "import escaped\nimport stolen\nimport forged\n",
        ),
        (
            ".venv/lib/python3.14/site-packages/safe-1.dist-info/RECORD",
            "escaped.py,,\nstolen/__init__.py,,\n",
        ),
        (".venv/lib/python3.14/site-packages/forged.py", ""),
        (
            ".venv/lib/python3.14/site-packages/forged-1.dist-info/marker",
            "",
        ),
    ]);
    let site = dir.path().join(".venv/lib/python3.14/site-packages");
    symlink(outside.path().join("escaped.py"), site.join("escaped.py")).unwrap();
    symlink(outside.path().join("stolen"), site.join("stolen")).unwrap();
    symlink(
        outside.path().join("metadata/RECORD"),
        site.join("forged-1.dist-info/RECORD"),
    )
    .unwrap();
    for record in collect(&dir, None, &[]).raw.imports.unwrap() {
        assert!(
            matches!(record.unresolved, Some(ImportReason::Missing)),
            "{}",
            record.specifier
        );
    }
}
