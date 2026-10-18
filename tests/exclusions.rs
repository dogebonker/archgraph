use std::path::{Path, PathBuf};
use std::process::Command;

use archgraph::collect::excludes::{Options, Policy, defaults, is_test};
use archgraph::model::Language;

fn root(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("exclusion-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}
fn paths(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| (*path).into()).collect()
}
fn keep(root: &Path, options: Options, candidates: &[&str]) -> Vec<String> {
    Policy::new(root, Language::Typescript, options)
        .unwrap()
        .select(&paths(candidates))
        .unwrap()
}
fn write(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}
fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-C",
        ])
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn defaults_cover_tests_generated_support_and_nested_build_directories() {
    let root = root("defaults");
    let candidates = [
        "src/code.ts",
        "root_test.go",
        "src/a.test.ts",
        "src/a.spec.ts",
        "tests/a.ts",
        "test/a.py",
        "__tests__/a.ts",
        "src/test_a.py",
        "conftest.py",
        "src/FooTest.java",
        "src/a.generated.ts",
        "src/a_pb.py",
        "api/a.pb.go",
        "dist/a.ts",
        "src/build/a.ts",
        "node_modules/a.ts",
        "vendor/a.go",
        ".venv/a.py",
        "fixtures/a.ts",
        "stories/a.ts",
        "src/a.stories.tsx",
        "examples/a.ts",
        "testdata/a.go",
    ];
    assert_eq!(
        keep(&root, Options::default(), &candidates),
        ["src/code.ts"]
    );
    let tests = keep(
        &root,
        Options {
            include_tests: true,
            ..Options::default()
        },
        &candidates,
    );
    assert!(tests.contains(&"root_test.go".into()));
    assert!(tests.contains(&"tests/a.ts".into()));
    assert!(!tests.contains(&"fixtures/a.ts".into()));
    assert!(!tests.contains(&"api/a.pb.go".into()));
    let generated = keep(
        &root,
        Options {
            include_generated: true,
            ..Options::default()
        },
        &candidates,
    );
    assert!(generated.contains(&"api/a.pb.go".into()));
    assert!(generated.contains(&"src/build/a.ts".into()));
    assert!(!generated.contains(&"tests/a.ts".into()));
    assert!(!generated.contains(&"examples/a.ts".into()));
    assert_eq!(
        keep(
            &root,
            Options {
                no_defaults: true,
                ..Options::default()
            },
            &candidates
        ),
        paths(&candidates)
    );
    assert!(defaults(Language::Rust).contains(&"**/target/**".into()));
    assert!(defaults(Language::Elixir).contains(&"**/_build/**".into()));
    assert!(defaults(Language::Elixir).contains(&"**/deps/**".into()));
}

#[test]
fn elixir_tests_and_build_outputs_share_selection_overrides() {
    let root = root("elixir-defaults");
    let candidates = paths(&[
        "lib/app.ex",
        "lib/app_test.exs",
        "test_helper.exs",
        "deps/dep/lib/dep.ex",
        "_build/dev/app.ex",
    ]);
    let policy = Policy::new(&root, Language::Elixir, Options::default()).unwrap();
    assert_eq!(policy.select(&candidates).unwrap(), paths(&["lib/app.ex"]));
    assert!(archgraph::collect::excludes::is_test("lib/app_test.exs"));
    assert!(archgraph::collect::excludes::is_test("test_helper.exs"));
    let policy = Policy::new(
        &root,
        Language::Elixir,
        Options {
            include_tests: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(
        policy.select(&candidates).unwrap(),
        paths(&["lib/app.ex", "lib/app_test.exs", "test_helper.exs"])
    );
    let policy = Policy::new(
        &root,
        Language::Elixir,
        Options {
            include_generated: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(
        policy.select(&candidates).unwrap(),
        paths(&["lib/app.ex", "deps/dep/lib/dep.ex", "_build/dev/app.ex"])
    );
}

#[test]
fn includes_restore_defaults_and_ignored_paths_but_explicit_excludes_win() {
    let root = root("precedence");
    write(&root, ".gitignore", "ignored/**\n");
    write(&root, ".archgraphignore", "custom/**\n");
    let options = Options {
        includes: paths(&["tests/**", "ignored/**", "custom/**"]),
        excludes: paths(&["tests/blocked.ts"]),
        cli_excludes: paths(&["custom/blocked.ts"]),
        ..Options::default()
    };
    let policy = Policy::new(&root, Language::Typescript, options).unwrap();
    assert!(policy.included("custom/allowed.ts"));
    assert_eq!(
        policy
            .select(&paths(&[
                "tests/allowed.ts",
                "tests/blocked.ts",
                "ignored/code.ts",
                "custom/allowed.ts",
                "custom/blocked.ts",
                "src/code.ts"
            ]))
            .unwrap(),
        paths(&[
            "tests/allowed.ts",
            "ignored/code.ts",
            "custom/allowed.ts",
            "src/code.ts"
        ])
    );
}

#[test]
fn git_syntax_handles_negations_escapes_anchoring_and_nested_ignore_files_without_a_repo() {
    let root = root("git-syntax");
    write(
        &root,
        ".gitignore",
        "ignored/*\n!ignored/keep.ts\n/root-only.ts\n",
    );
    write(&root, "nested/.gitignore", "*.ts\n!keep.ts\n");
    write(
        &root,
        ".archgraphignore",
        "custom/*.ts\n!custom/keep.ts\nspace\\ name.ts\n\\#literal.ts\ncache/\n",
    );
    assert_eq!(
        keep(
            &root,
            Options::default(),
            &[
                "ignored/drop.ts",
                "ignored/keep.ts",
                "root-only.ts",
                "nested/root-only.ts",
                "nested/keep.ts",
                "nested/deep/code.ts",
                "custom/drop.ts",
                "custom/keep.ts",
                "space name.ts",
                "#literal.ts",
                "cache/deep/code.ts",
                "src/code.ts"
            ]
        ),
        paths(&[
            "ignored/keep.ts",
            "nested/keep.ts",
            "custom/keep.ts",
            "src/code.ts"
        ])
    );
}

#[test]
fn ignores_honor_tracked_files_and_monorepo_ancestor_rules() {
    let root = root("tracked");
    git(&root, &["init", "--quiet"]);
    write(&root, "tracked.ts", "export const Tracked = 1;");
    git(&root, &["add", "tracked.ts"]);
    write(&root, ".gitignore", "tracked.ts\npackages/app/ignored.ts\n");
    assert!(keep(&root, Options::default(), &["tracked.ts"]).is_empty());
    std::fs::create_dir_all(root.join("packages/app")).unwrap();
    write(&root, "packages/app/.archgraphignore", "/local.ts\n");
    assert_eq!(
        keep(
            &root.join("packages/app"),
            Options::default(),
            &["ignored.ts", "local.ts", "src/kept.ts"]
        ),
        paths(&["src/kept.ts"])
    );
}

#[test]
fn bulk_ignore_queries_do_not_deadlock_on_pipe_capacity() {
    let root = root("bulk");
    write(&root, ".gitignore", "ignored/\n");
    let candidates: Vec<_> = (0..12_000)
        .map(|i| format!("ignored/long-module-name-{i}.ts"))
        .collect();
    let policy = Policy::new(&root, Language::Typescript, Options::default()).unwrap();
    assert!(policy.select(&candidates).unwrap().is_empty());
}

#[test]
fn classification_does_not_mark_ordinary_names_as_tests() {
    for path in [
        "tests/a.ts",
        "src/a_test.go",
        "src/test_logic.py",
        "src/a.spec.ts",
        "src/FooTests.java",
    ] {
        assert!(is_test(path), "{path}");
    }
    for path in [
        "src/contest.ts",
        "src/testimony.py",
        "src/Foo.java",
        "src/example.ts",
    ] {
        assert!(!is_test(path), "{path}");
    }
}

#[test]
fn invalid_patterns_and_unreadable_archgraphignore_are_errors() {
    let root = root("errors");
    assert!(
        Policy::new(
            &root,
            Language::Typescript,
            Options {
                includes: paths(&["["]),
                ..Options::default()
            }
        )
        .is_err()
    );
    let policy = Policy::new(&root, Language::Typescript, Options::default()).unwrap();
    assert!(policy.select(&["src/a\0b.ts".into()]).is_err());
    std::fs::create_dir(root.join(".archgraphignore")).unwrap();
    assert!(Policy::new(&root, Language::Typescript, Options::default()).is_err());
}

#[test]
fn layout_accepts_selection_patterns_and_rejects_invalid_globs() {
    let root = root("rules");
    write(
        &root,
        "layout.yaml",
        "version: 1\nexclude: ['src/generated/**']\ninclude: ['src/generated/kept.ts']\n",
    );
    let rules = archgraph::rules::load(&root.join("layout.yaml")).unwrap();
    assert_eq!(rules.doc.exclude, ["src/generated/**"]);
    assert_eq!(rules.doc.include, ["src/generated/kept.ts"]);
    write(&root, "layout.yaml", "version: 1\ninclude: ['[']\n");
    assert!(archgraph::rules::load(&root.join("layout.yaml")).is_err());
}

#[test]
fn language_discovery_uses_the_same_restored_selection_policy() {
    use archgraph::collect::{detect_language, detect_language_with_options};
    let root = root("detection");
    write(&root, "src/build/only.ts", "export const Built = 1;");
    assert!(detect_language(&root, "src").is_err());
    assert_eq!(
        detect_language_with_options(
            &root,
            "src",
            Options {
                include_generated: true,
                ..Options::default()
            }
        )
        .unwrap(),
        Language::Typescript
    );
    assert_eq!(
        detect_language_with_options(
            &root,
            "src",
            Options {
                includes: paths(&["src/build/only.ts"]),
                ..Options::default()
            }
        )
        .unwrap(),
        Language::Typescript
    );
    write(&root, ".archgraphignore", "src/build/only.ts\n");
    assert!(
        detect_language_with_options(
            &root,
            "src",
            Options {
                include_generated: true,
                ..Options::default()
            }
        )
        .is_err()
    );
    assert_eq!(
        detect_language_with_options(
            &root,
            "src",
            Options {
                includes: paths(&["src/build/only.ts"]),
                ..Options::default()
            }
        )
        .unwrap(),
        Language::Typescript
    );
}

#[cfg(unix)]
#[test]
fn broken_and_escaping_archgraphignore_symlinks_are_errors() {
    let root = root("ignore-symlinks");
    std::os::unix::fs::symlink("missing", root.join(".archgraphignore")).unwrap();
    assert!(Policy::new(&root, Language::Typescript, Options::default()).is_err());
    std::fs::remove_file(root.join(".archgraphignore")).unwrap();
    let outside = root.with_extension("outside-ignore");
    std::fs::write(&outside, "*.ts\n").unwrap();
    std::os::unix::fs::symlink(&outside, root.join(".archgraphignore")).unwrap();
    assert!(Policy::new(&root, Language::Typescript, Options::default()).is_err());
}
