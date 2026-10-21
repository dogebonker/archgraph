use archgraph::collect::{
    excludes::{Options, Policy},
    native::java,
};
use archgraph::model::{ImportReason, Language};
use std::fs;
use std::path::{Path, PathBuf};
struct Fixture(PathBuf);
impl Fixture {
    fn path(&self) -> &Path {
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
    let root = Fixture(std::env::temp_dir().join(format!(
        "archgraph-native-java-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )));
    fs::create_dir_all(root.path()).unwrap();
    for (path, source) in files {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    root
}
fn collect(root: &Path) -> archgraph::collect::native::Output {
    let policy = Policy::new(root, Language::Java, Options::default()).unwrap();
    java::collect(root, &policy).unwrap()
}
#[test]
fn native_java_resolves_reference_fixture_without_jdk() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-resolve");
    let output = collect(&root);
    let edges: Vec<_> = output
        .raw
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    assert_eq!(
        edges,
        [
            (
                "src/main/java/demo/model/Factory.java",
                "src/main/java/demo/model/Order.java"
            ),
            (
                "src/main/java/demo/service/Checkout.java",
                "src/main/java/demo/model/Factory.java"
            ),
            (
                "src/main/java/demo/service/Checkout.java",
                "src/main/java/demo/model/Order.java"
            ),
            (
                "src/main/java/demo/web/Controller.java",
                "src/main/java/demo/model/Order.java"
            ),
            (
                "src/main/java/demo/web/Controller.java",
                "src/main/java/demo/service/Checkout.java"
            ),
        ]
    );
    assert!(output.raw.files.iter().any(|f| f.ends_with("Test.java")));
    assert!(
        !output
            .raw
            .edges
            .iter()
            .any(|e| e.from.ends_with("Test.java") || e.to.ends_with("Test.java"))
    );
}
#[test]
fn native_java_nested_types_wildcards_and_qualified_static_receivers() {
    let root = fixture(&[
        (
            "model/Outer.java",
            "package model; public class Outer { public static class Inner {} public static int VALUE=1; }",
        ),
        (
            "model/Other.java",
            "package model; public record Other(int id) {}",
        ),
        (
            "use/Use.java",
            "package use; import model.*; class Use { Outer.Inner inner; Other other; int n=model.Outer.VALUE; }",
        ),
    ]);
    let output = collect(root.path());
    assert_eq!(output.raw.edges.len(), 2);
    assert!(output.raw.edges.iter().any(|e| e.to == "model/Outer.java"));
    assert!(output.raw.edges.iter().any(|e| e.to == "model/Other.java"));
    assert!(
        output
            .symbols
            .exports
            .iter()
            .any(|s| s.file == "model/Outer.java" && s.name == "Inner")
    );
    assert!(!output.symbols.complete);
}
#[test]
fn native_java_ambiguous_wildcard_and_missing_classpath_stay_unresolved() {
    let root = fixture(&[
        ("a/Item.java", "package a; public class Item {}"),
        ("b/Item.java", "package b; public class Item {}"),
        (
            "Use.java",
            "import a.*; import b.*; import missing.library.Dep; class Use { Item item; Dep dependency; }",
        ),
    ]);
    let output = collect(root.path());
    let imports = output.raw.imports.unwrap();
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "Item" && matches!(i.unresolved, Some(ImportReason::Alias)))
    );
    assert!(imports.iter().any(|i| i.specifier == "missing.library.Dep"
        && matches!(i.unresolved, Some(ImportReason::Missing))));
    assert!(!output.raw.edges.iter().any(|e| e.to.ends_with("Item.java")));
}
#[test]
fn native_java_jdk_catalog_excluded_targets_and_no_execution() {
    let root = fixture(&[
        (
            "Use.java",
            "import java.util.List; import java.fake.Missing; import javax.enterprise.Inject; import demo.HiddenTest; class Use { static { if(true) throw new RuntimeException(\"never execute\"); } List<String> list; HiddenTest hidden; }",
        ),
        (
            "HiddenTest.java",
            "package demo; public class HiddenTest {}",
        ),
        ("module-info.java", "module demo { requires java.base; }"),
        (
            "build.gradle",
            "throw new RuntimeException('never execute')",
        ),
    ]);
    let output = collect(root.path());
    let imports = output.raw.imports.unwrap();
    assert!(
        imports
            .iter()
            .any(|i| i.external.as_deref() == Some("java.util"))
    );
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "java.fake.Missing" && i.unresolved.is_some())
    );
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "javax.enterprise.Inject" && i.unresolved.is_some())
    );
    assert!(imports.iter().any(|i| i.targets == ["HiddenTest.java"]));
    assert!(output.raw.files.contains(&"HiddenTest.java".into()));
    assert!(!output.raw.edges.iter().any(|e| e.to == "HiddenTest.java"));
    assert!(
        output
            .symbols
            .limitations
            .iter()
            .any(|s| s.contains("module-info"))
    );
    assert!(!root.path().join("Use.class").exists());
}
#[test]
fn native_java_syntax_errors_keep_valid_imports_and_generic_parameters_are_local() {
    let root = fixture(&[
        ("model/T.java", "package model; public class T {}"),
        (
            "Use.java",
            "import model.T; class Use<T> { T value; broken( }",
        ),
    ]);
    let output = collect(root.path());
    assert!(
        output
            .raw
            .imports
            .as_ref()
            .unwrap()
            .iter()
            .any(|i| i.specifier == "model.T" && i.targets == ["model/T.java"])
    );
    // `T` in the class is the type parameter, so the import is unused and gives no edge.
    assert!(output.raw.edges.is_empty());
    assert!(
        output
            .symbols
            .limitations
            .iter()
            .any(|s| s.contains("syntax errors"))
    );
}
#[cfg(unix)]
#[test]
fn native_java_does_not_follow_symlinks_outside_scan_root() {
    let outside = fixture(&[("Outside.java", "public class Outside {}")]);
    let root = fixture(&[("Local.java", "public class Local {}")]);
    std::os::unix::fs::symlink(
        outside.path().join("Outside.java"),
        root.path().join("Outside.java"),
    )
    .unwrap();
    let output = collect(root.path());
    assert_eq!(output.raw.files, ["Local.java"]);
}
#[test]
fn native_java_static_members_require_declaring_source_proof() {
    let root = fixture(&[
        (
            "model/Factory.java",
            "package model; public class Factory { public static int VALUE=1; public static int create(){ return 1; } }",
        ),
        (
            "Use.java",
            "import static model.Factory.VALUE; import static model.Factory.create; import static model.Factory.missing; class Use { int n=VALUE+create(); }",
        ),
    ]);
    let output = collect(root.path());
    let imports = output.raw.imports.unwrap();
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "model.Factory.VALUE" && i.targets == ["model/Factory.java"])
    );
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "model.Factory.create" && i.targets == ["model/Factory.java"])
    );
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "model.Factory.missing" && i.unresolved.is_some())
    );
}
#[test]
fn native_java_local_classes_do_not_become_importable_member_types() {
    let root = fixture(&[
        (
            "model/Outer.java",
            "package model; public class Outer { void run() { class Local {} } }",
        ),
        (
            "Use.java",
            "import model.Outer.Local; class Use { Local value; }",
        ),
    ]);
    let output = collect(root.path());
    assert!(
        output
            .raw
            .imports
            .unwrap()
            .iter()
            .any(|i| i.specifier == "model.Outer.Local" && i.unresolved.is_some())
    );
    assert!(output.raw.edges.is_empty());
}
#[test]
fn native_java_excluded_copies_do_not_make_included_types_ambiguous() {
    let util = "package p; public class Util { public static void helper(){} }";
    let root = fixture(&[
        ("src/main/java/p/Util.java", util),
        ("target/generated-sources/delombok/p/Util.java", util),
        (
            "src/main/java/app/Use.java",
            "package app; import p.Util; class Use { Util util; }",
        ),
        (
            "src/main/java/app/Wild.java",
            "package app; import static p.Util.*; class Wild { int count; void run(){ String label = \"x\"; helper(); } }",
        ),
    ]);
    let output = collect(root.path());
    let record = output
        .raw
        .imports
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.specifier == "p.Util")
        .unwrap();
    assert_eq!(record.targets, ["src/main/java/p/Util.java"]);
    assert!(record.unresolved.is_none());
    assert!(output.raw.edges.iter().any(|e| {
        e.from == "src/main/java/app/Use.java" && e.to == "src/main/java/p/Util.java"
    }));
    let names: std::collections::BTreeSet<_> = output
        .symbols
        .imports
        .iter()
        .filter(|i| i.from == "src/main/java/app/Wild.java")
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(names, std::collections::BTreeSet::from(["helper"]));
}
#[test]
fn native_java_method_reference_qualifier_is_a_use_of_the_import() {
    let root = fixture(&[
        (
            "model/Order.java",
            "package model; public class Order { public static void touch(){} }",
        ),
        (
            "Ref.java",
            "import model.Order; class Ref { Runnable r = Order::touch; }",
        ),
    ]);
    let output = collect(root.path());
    let edges: Vec<_> = output
        .raw
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    assert_eq!(edges, [("Ref.java", "model/Order.java")]);
}
