//! Runs the real collectors. Needs depcruise 18.4.0 (with typescript 6.0.3 beside it), a
//! python with grimp 3.17 and go 1.27.1, from BOUNDRAIL_DEPCRUISE / BOUNDRAIL_PYTHON /
//! BOUNDRAIL_GO or PATH, plus node and git (BOUNDRAIL_NODE / BOUNDRAIL_GIT or PATH). The Rust
//! tests need rustup, whose active toolchain (1.95.0) has rust-analyzer and rust-src, and cc.

mod common;

use std::path::{Path, PathBuf};
use std::{env, fs};

use common::{boundrail, fixture, scan, scan_recorded, stable, validate};
use serde_json::json;

fn tool(var: &str, name: &str) -> PathBuf {
    env::var_os(var)
        .map(PathBuf::from)
        .or_else(|| {
            env::split_paths(&env::var_os("PATH")?)
                .map(|d| d.join(name))
                .find(|p| p.is_file())
        })
        .unwrap_or_else(|| panic!("set {var} or put {name} on PATH"))
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
#[ignore = "needs depcruise 18.4.0 / grimp 3.17; run with --ignored"]
fn depcruise_live_matches_recorded() {
    // `scan` runs `npx --no-install depcruise`, which finds the tool in the scanned project's
    // node_modules, so each fixture is copied next to a node_modules link to the pinned install.
    let bin = fs::canonicalize(tool("BOUNDRAIL_DEPCRUISE", "depcruise")).unwrap();
    let node_modules = bin
        .ancestors()
        .find(|p| p.ends_with("node_modules"))
        .unwrap()
        .to_owned();
    for name in [
        "ts-basic",
        "ts-cycles",
        "ts-excludes",
        "ts-fda",
        "ts-modmono",
        "ts-hexagonal",
        "ts-sink",
        "ts-misplaced",
        "ts-internals",
        "ts-sdp",
        "ts-seams",
        "ts-elided",
    ] {
        let copy = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("live")
            .join(name);
        let _ = fs::remove_dir_all(&copy);
        copy_dir(&fixture(name), &copy);
        std::os::unix::fs::symlink(&node_modules, copy.join("node_modules")).unwrap();

        let (live, live_out, _) = scan(&[copy.to_str().unwrap()]);
        let (_, recorded_out, _) = scan_recorded(name, &[]);
        assert_eq!(stable(&live_out), stable(&recorded_out), "{name}");

        if name == "ts-basic" {
            let projected = common::without_import_metadata(&live["edges"]);
            let edges = projected.as_array().unwrap();
            let alias = json!({ "from": "src/main.ts", "to": "src/app/shell/run.ts", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null });
            let type_only = json!({ "from": "src/app/shell/run.ts", "to": "src/core/model/types.ts",
                                    "kind": "type", "weight": 0.1, "verdict": null, "rule": null });
            assert!(edges.contains(&alias) && edges.contains(&type_only));
        }
    }
}

#[test]
#[ignore = "needs depcruise 18.4.0 / grimp 3.17; run with --ignored"]
fn depcruise_path_runs_a_pinned_install_outside_the_scanned_repository() {
    // No node_modules beside the fixture and only node and git on PATH: npx could not find
    // depcruise, so a matching scan proves --depcruise ran.
    let bin = fs::canonicalize(tool("BOUNDRAIL_DEPCRUISE", "depcruise")).unwrap();
    let dirs = [tool("BOUNDRAIL_NODE", "node"), tool("BOUNDRAIL_GIT", "git")]
        .map(|p| p.parent().unwrap().to_owned());
    assert!(dirs.iter().all(|d| !d.join("depcruise").exists()));
    let dir = fixture("ts-basic");
    assert!(!dir.join("node_modules").exists());
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_boundrail"))
        .args([
            "scan",
            dir.to_str().unwrap(),
            "--depcruise",
            bin.to_str().unwrap(),
        ])
        .env("PATH", env::join_paths(&dirs).unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let live_out = String::from_utf8(out.stdout).unwrap();
    validate(&serde_json::from_str(&live_out).unwrap());
    let (_, recorded_out, _) = scan_recorded("ts-basic", &[]);
    assert_eq!(stable(&live_out), stable(&recorded_out));
}

#[test]
#[ignore = "needs depcruise 18.4.0 / grimp 3.17; run with --ignored"]
fn grimp_live_matches_recorded() {
    let python = tool("BOUNDRAIL_PYTHON", "python3");
    for (name, package) in [
        ("py-cycles", "mypkg"),
        ("py-clean", "src/app"),
        ("py-deferred", "deferred"),
    ] {
        let dir = fixture(name);
        let (_, live_out, _) = scan(&[dir.to_str().unwrap(), "--python", python.to_str().unwrap()]);
        let (_, recorded_out, _) = scan_recorded(name, &[]);
        assert_eq!(stable(&live_out), stable(&recorded_out), "{name}");
        assert!(
            !dir.join(package).join("__pycache__").exists(),
            "grimp wrote into {name}"
        );
    }
}

#[test]
#[ignore = "needs depcruise 18.4.0 / grimp 3.17; run with --ignored"]
fn grimp_never_runs_code_from_the_scanned_repository() {
    // A json.py in the source root would shadow the stdlib module if the source root came first
    // on sys.path.
    let python = tool("BOUNDRAIL_PYTHON", "python3");
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("planted-json");
    let _ = fs::remove_dir_all(&root);
    let marker = root.join("RAN");
    fs::create_dir_all(root.join("src/pkg")).unwrap();
    fs::write(
        root.join("src/json.py"),
        format!("open({:?}, 'w').write('ran')\n", marker.to_str().unwrap()),
    )
    .unwrap();
    fs::write(root.join("src/pkg/__init__.py"), "").unwrap();
    fs::write(root.join("src/pkg/a.py"), "from pkg import b\n").unwrap();
    fs::write(root.join("src/pkg/b.py"), "").unwrap();
    let (doc, _, _) = scan(&[root.to_str().unwrap(), "--python", python.to_str().unwrap()]);
    assert!(!marker.exists(), "the planted json.py ran");
    assert_eq!(
        common::without_import_metadata(&doc["edges"]),
        json!([{ "from": "src/pkg/a.py", "to": "src/pkg/b.py", "kind": "runtime", "weight": 1.0, "verdict": null,
                 "rule": null }])
    );
}

#[test]
#[ignore = "needs go 1.27.1; run with --ignored"]
fn go_live_matches_recorded() {
    let go = tool("BOUNDRAIL_GO", "go");
    let go = go.to_str().unwrap();
    for name in [
        "go-resolve",
        "go-cycles",
        "go-misplaced",
        "go-layers",
        "go-internals",
        "go-excludes",
    ] {
        let dir = fixture(name);
        for flags in [&[][..], &["--include-tests"], &["--no-default-excludes"]] {
            let args = [&[dir.to_str().unwrap(), "--go", go][..], flags].concat();
            let (_, live_out, _) = scan(&args);
            let (_, recorded_out, _) = scan_recorded(name, flags);
            assert_eq!(stable(&live_out), stable(&recorded_out), "{name} {flags:?}");
        }
        // go list -mod=readonly never writes go.sum (or go.mod).
        assert!(!dir.join("go.sum").exists(), "go wrote into {name}");
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let recordings = ["go-misplaced", "go-excludes"].map(|name| {
            let (recorded, _, _) = scan_recorded(name, &[]);
            (fixture(name), recorded)
        });
        for platform in ["darwin", "linux", "windows"] {
            // Cross-list the fixture while compiling and running the helper for this host.
            let script = common::write_tmp(
                &format!("go-live-platform/{platform}"),
                &format!(
                    "#!/bin/sh\nif [ \"$1\" = list ]; then\n  export GOOS={platform}\nfi\nexec '{}' \"$@\"\n",
                    go.replace('\'', "'\\''")
                ),
            );
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
            for (dir, recorded) in &recordings {
                let (live, _, _) = scan(&[dir.to_str().unwrap(), "--go", &script]);
                assert_eq!(
                    live["source"]["collector"]["bound"],
                    recorded["source"]["collector"]["bound"],
                    "{} platform variants under GOOS={platform}",
                    dir.display()
                );
            }
        }
    }
}

/// A module under the test temp dir with `go_mod` and one package `a` of `files`.
fn go_module(name: &str, go_mod: &str, files: &[(&str, String)]) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("a")).unwrap();
    fs::write(root.join("go.mod"), go_mod).unwrap();
    for (file, text) in files {
        fs::write(root.join(file), text).unwrap();
    }
    root
}

#[test]
#[ignore = "needs go 1.27.1; run with --ignored"]
fn go_never_downloads_a_toolchain() {
    let go = tool("BOUNDRAIL_GO", "go");
    let a = [("a/a.go", "package a\n".to_owned())];
    // GOTOOLCHAIN=local ignores a toolchain line: the scan runs with the local Go, offline.
    let root = go_module(
        "go-toolchain-line",
        "module example.com/t\n\ngo 1.21\n\ntoolchain go1.99.0\n",
        &a,
    );
    let (doc, _, _) = scan(&[root.to_str().unwrap(), "--go", go.to_str().unwrap()]);
    assert_eq!(doc["source"]["collector"]["version"], "1.27.1");
    // A go line newer than the local Go cannot be met without a download: the scan fails and
    // says why.
    let root = go_module("go-newer-go-line", "module example.com/t\n\ngo 1.99\n", &a);
    let out = boundrail(&["scan", root.to_str().unwrap(), "--go", go.to_str().unwrap()]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("go.mod requires go >= 1.99"), "{stderr}");
    assert!(stderr.contains("boundrail never downloads one"), "{stderr}");
}

#[test]
#[ignore = "needs go 1.27.1; run with --ignored"]
fn go_scan_runs_no_repository_code() {
    // Every place repository code could run from: go:generate, init and main, a test's TestMain,
    // and a go.work the go command would read (its go line would fail the scan).
    let go = tool("BOUNDRAIL_GO", "go");
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("go-planted");
    let marker = |what: &str| root.join(format!("RAN-{what}"));
    let write = |what: &str| {
        format!(
            "os.WriteFile({:?}, nil, 0o644)",
            marker(what).to_str().unwrap()
        )
    };
    let files = [
        (
            "a/a.go",
            format!(
                "//go:generate touch {}\npackage a\n\nimport \"os\"\n\nfunc init() {{ {} }}\n",
                marker("generate").to_str().unwrap(),
                write("init")
            ),
        ),
        (
            "main.go",
            format!(
                "package main\n\nimport (\n\t\"os\"\n\n\t_ \"example.com/p/a\"\n)\n\nfunc main() {{ {} }}\n",
                write("main")
            ),
        ),
        (
            "a/a_test.go",
            format!(
                "package a\n\nimport (\n\t\"os\"\n\t\"testing\"\n)\n\nfunc TestMain(m *testing.M) {{ {} }}\n",
                write("test")
            ),
        ),
        ("go.work", "go 1.99\n\nuse .\n".to_owned()),
    ];
    let root = go_module("go-planted", "module example.com/p\n\ngo 1.21\n", &files);
    let (doc, _, _) = scan(&[
        root.to_str().unwrap(),
        "--go",
        go.to_str().unwrap(),
        "--include-tests",
    ]);
    // The blank import's edge, to package a's anchor file.
    assert_eq!(
        common::without_import_metadata(&doc["edges"]),
        json!([
            { "from": "main.go", "to": "a/a.go", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null }
        ])
    );
    for what in ["generate", "init", "main", "test"] {
        assert!(!marker(what).exists(), "repository code ran: {what}");
    }
}

#[test]
#[ignore = "needs go 1.27.1; run with --ignored"]
fn go_scan_ignores_a_stale_vendor_directory() {
    // vendor/modules.txt disagrees with go.mod: -mod=vendor would fail the whole scan, and the
    // scan needs nothing from vendor/.
    let go = tool("BOUNDRAIL_GO", "go");
    let files = [
        (
            "a/a.go",
            "package a\n\nimport \"example.org/dep\"\n\nvar X = dep.Y\n".to_owned(),
        ),
        (
            "vendor/modules.txt",
            "# example.org/dep v1.0.0\n## explicit\nexample.org/dep\n".to_owned(),
        ),
        (
            "vendor/example.org/dep/dep.go",
            "package dep\n\nvar Y = 1\n".to_owned(),
        ),
    ];
    let root = go_module(
        "go-stale-vendor",
        "module example.com/p\n\ngo 1.21\n\nrequire example.org/dep v1.2.3\n",
        &[],
    );
    for (file, text) in &files {
        fs::create_dir_all(root.join(file).parent().unwrap()).unwrap();
        fs::write(root.join(file), text).unwrap();
    }
    let (doc, _, _) = scan(&[root.to_str().unwrap(), "--go", go.to_str().unwrap()]);
    assert_eq!(
        doc["files"],
        json!([{ "path": "a/a.go", "module": "a", "barrel": false }])
    );
}

#[test]
#[ignore = "needs go 1.27.1; run with --ignored"]
fn go_scan_replaces_the_users_goflags() {
    // -toolexec=/bin/false would fail every compile, the helper's included.
    let go = tool("BOUNDRAIL_GO", "go");
    let root = go_module(
        "go-hostile-goflags",
        "module example.com/t\n\ngo 1.21\n",
        &[("a/a.go", "package a\n".to_owned())],
    );
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_boundrail"))
        .args(["scan", root.to_str().unwrap(), "--go", go.to_str().unwrap()])
        .env("GOFLAGS", "-toolexec=/bin/false")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["files"][0]["path"], "a/a.go");
}

#[test]
#[ignore = "needs go 1.27.1; run with --ignored"]
fn go_scan_reads_a_module_go_list_rejects_with_go_build() {
    // legacy/go.mod has a require without a version: go list fails in that module, so its
    // packages are read with go/build, with a warning; --exclude leaves the module out entirely.
    let go = tool("BOUNDRAIL_GO", "go");
    let root = go_module(
        "go-list-fails",
        "module example.com/p\n\ngo 1.21\n",
        &[(
            "a/a.go",
            "package a\n\nfunc A() int { return 1 }\n".to_owned(),
        )],
    );
    fs::create_dir_all(root.join("legacy/sub")).unwrap();
    fs::write(
        root.join("legacy/go.mod"),
        "module example.com/legacy // the old name\n\ngo 1.21\n\nrequire example.org/x\n",
    )
    .unwrap();
    // A nested go.mod without a module line is skipped, and still bounds legacy: its files are
    // no part of legacy.
    fs::create_dir_all(root.join("legacy/nested")).unwrap();
    fs::write(root.join("legacy/nested/go.mod"), "go 1.21\n").unwrap();
    fs::write(root.join("legacy/nested/n.go"), "package nested\n").unwrap();
    fs::write(
        root.join("legacy/l.go"),
        "package legacy\n\nimport \"example.com/legacy/sub\"\n\nvar L = sub.S\n",
    )
    .unwrap();
    fs::write(root.join("legacy/sub/s.go"), "package sub\n\nvar S = 1\n").unwrap();
    let go = go.to_str().unwrap();
    let (doc, _, stderr) = scan(&[root.to_str().unwrap(), "--go", go]);
    assert!(
        stderr.contains("reading its packages with go/build"),
        "{stderr}"
    );
    let paths: Vec<&str> = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["a/a.go", "legacy/l.go", "legacy/sub/s.go"]);
    assert_eq!(
        common::without_import_metadata(&doc["edges"]),
        json!([{ "from": "legacy/l.go", "to": "legacy/sub/s.go", "kind": "runtime", "weight": 1.0, "verdict": null,
                 "rule": null }])
    );
    let (doc, _, stderr) = scan(&[root.to_str().unwrap(), "--go", go, "--exclude", "legacy/**"]);
    assert!(!stderr.contains("warning"), "{stderr}");
    assert_eq!(
        doc["files"],
        json!([{ "path": "a/a.go", "module": "a", "barrel": false }])
    );
}

/// An empty directory for a Rust repository, under the system temp dir: Boundrail's own
/// rust-toolchain.toml (1.95.0 by name) would be refused from any directory below it.
fn rust_dir(name: &str) -> PathBuf {
    let dir = env::temp_dir().join("boundrail-live-rust").join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::canonicalize(&dir).unwrap()
}

fn write_files(root: &Path, files: &[(&str, &str)]) {
    for (file, text) in files {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

const LOCK_W: &str = "version = 4\n\n[[package]]\nname = \"w\"\nversion = \"0.1.0\"\n";

/// A library crate `w` with a Cargo.lock at `rust_dir(name)/repo`, plus `files`.
fn rust_crate(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = rust_dir(name).join("repo");
    write_files(
        &root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"w\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            ),
            ("Cargo.lock", LOCK_W),
            ("src/lib.rs", "pub mod a;\npub fn top() -> u32 { a::f() }\n"),
            ("src/a.rs", "pub fn f() -> u32 { 1 }\n"),
        ],
    );
    write_files(&root, files);
    root
}

/// Where hostile code reports that it ran: `dylib` writes `dylib-<pid>` into `dir` from any
/// process that loads it, and `script` writes `script-<pid>` and runs its arguments.
struct Markers {
    dir: PathBuf,
    dylib: String,
    script: String,
}

impl Markers {
    fn new(name: &str) -> Markers {
        let base = rust_dir(&format!("{name}-markers"));
        let dir = base.join("fired");
        fs::create_dir(&dir).unwrap();
        let ext = if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        };
        let dylib = base.join(format!("libmarker.{ext}"));
        let status = std::process::Command::new("cc")
            .args(["-shared", "-fPIC", "-o"])
            .arg(&dylib)
            .arg(format!("-DMARKER_DIR=\"{}\"", dir.display()))
            .arg(fixture("rust-hostile/marker.c"))
            .status()
            .unwrap();
        assert!(status.success());
        let script = base.join("marker.sh");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\ntouch '{}/script-'$$\nexec \"$@\"\n",
                dir.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let s = |p: PathBuf| p.to_str().unwrap().to_owned();
        Markers {
            dir,
            dylib: s(dylib),
            script: s(script),
        }
    }

    /// `text` with `{dylib}` and `{script}` filled in.
    fn fill(&self, text: &str) -> String {
        text.replace("{dylib}", &self.dylib)
            .replace("{script}", &self.script)
    }

    fn fired(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

/// Every directory and file under `root` with its contents.
fn tree(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path.clone());
                out.insert(path, Vec::new());
            } else {
                out.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn host() -> String {
    let out = std::process::Command::new("rustc")
        .arg("-vV")
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .unwrap()
        .to_owned()
}

/// `scan --language rust` on `root`, with and without `--rust-build-scripts`: each must exit 2
/// with `want` in its message.
fn rust_refused(root: &Path, want: &str) {
    for extra in [&[][..], &["--rust-build-scripts"]] {
        let root = root.to_str().unwrap();
        let out = boundrail(&[&["scan", root, "--language", "rust"], extra].concat());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains(want), "{want} not in: {stderr}");
    }
}

/// A Cargo config at `file` (relative to the repository, `..` for an ancestor) setting `key`
/// is refused before cargo runs: no marker, no change to the tree.
fn assert_config_refused(name: &str, file: &str, config: &str, key: &str) {
    let m = Markers::new(name);
    let root = rust_crate(name, &[]);
    write_files(&root, &[(file, &m.fill(config))]);
    let before = tree(root.parent().unwrap());
    rust_refused(&root, &format!("sets `{key}`"));
    assert_eq!(m.fired(), Vec::<String>::new());
    assert_eq!(tree(root.parent().unwrap()), before);
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_scan_resolves_cross_file_calls_without_changing_the_repository() {
    let root = rust_crate(
        "rust-edges",
        &[
            ("src/lib.rs", "pub mod a;\npub mod b;\n"),
            ("src/a.rs", "pub fn a() -> u32 { crate::b::b() }\n"),
            ("src/b.rs", "pub fn b() -> u32 { 1 }\n"),
        ],
    );
    let before = tree(&root);
    let (doc, _, _) = scan(&[root.to_str().unwrap(), "--language", "rust"]);
    assert_eq!(
        common::without_import_metadata(&doc["edges"]),
        json!([
            { "from": "src/a.rs", "to": "src/b.rs", "kind": "runtime", "weight": 1.0, "verdict": null,
              "rule": null },
            { "from": "src/lib.rs", "to": "src/a.rs", "kind": "runtime", "weight": 1.0, "verdict": null,
              "rule": null },
            { "from": "src/lib.rs", "to": "src/b.rs", "kind": "runtime", "weight": 1.0, "verdict": null,
              "rule": null }
        ])
    );
    assert_eq!(tree(&root), before);
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_history_scans_real_commits_and_preserves_the_checkout() {
    let root = rust_crate(
        "rust-history",
        &[("src/lib.rs", "pub mod a;\npub fn top() -> u32 { 1 }\n")],
    );
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args([
                "-c",
                "user.name=boundrail-test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "before"]);
    let first = git(&["rev-parse", "HEAD"]);
    fs::write(
        root.join("src/a.rs"),
        "pub fn f() -> u32 { crate::top() }\n",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "dependency on parent"]);
    let second = git(&["rev-parse", "HEAD"]);
    fs::write(root.join("src/a.rs"), "dirty source must survive\n").unwrap();
    let before = tree(&root);
    let reports = root.parent().unwrap().join("reports");
    let out = boundrail(&[
        "history",
        "--sample",
        "commits",
        root.to_str().unwrap(),
        "--language",
        "rust",
        "--limit",
        "2",
        "--out-dir",
        reports.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(reports.join("history.json")).unwrap()).unwrap();
    validate(&doc);
    let snapshots = doc["snapshots"].as_array().unwrap();
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0]["commit"], first);
    assert_eq!(snapshots[1]["commit"], second);
    assert!(snapshots[0]["diff"].is_null());
    assert_eq!(snapshots[0]["graph"]["edges"].as_array().unwrap().len(), 1);
    assert_eq!(snapshots[1]["graph"]["edges"].as_array().unwrap().len(), 2);
    assert_eq!(
        snapshots[1]["diff"]["new_cycles"].as_array().unwrap().len(),
        1
    );
    for snapshot in snapshots {
        validate(&snapshot["graph"]);
        assert_eq!(snapshot["graph"]["source"]["language"], "rust");
        assert_eq!(
            snapshot["graph"]["source"]["git"]["commit"],
            snapshot["commit"]
        );
    }
    assert!(reports.join("history.html").is_file());
    assert_eq!(tree(&root), before);
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_scan_runs_no_build_script_or_proc_macro() {
    let m = Markers::new("rust-codegen");
    let root = rust_dir("rust-codegen");
    let fired = m.dir.display().to_string();
    write_files(
        &root,
        &[
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"app\", \"pm\"]\nresolver = \"2\"\n",
            ),
            (
                "Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n\
                 dependencies = [\"pm\"]\n\n[[package]]\nname = \"pm\"\nversion = \"0.1.0\"\n",
            ),
            (
                "app/Cargo.toml",
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
                 [dependencies]\npm = { path = \"../pm\" }\n",
            ),
            (
                "app/build.rs",
                &format!("fn main() {{ std::fs::write(\"{fired}/build-rs\", \"\").unwrap(); }}\n"),
            ),
            ("app/src/lib.rs", "pm::mark!();\npub fn f() {}\n"),
            (
                "pm/Cargo.toml",
                "[package]\nname = \"pm\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
                 [lib]\nproc-macro = true\n",
            ),
            (
                "pm/src/lib.rs",
                &format!(
                    "use proc_macro::TokenStream;\n#[proc_macro]\n\
                     pub fn mark(_: TokenStream) -> TokenStream {{\n\
                     std::fs::write(\"{fired}/proc-macro\", \"\").unwrap();\n\
                     TokenStream::new()\n}}\n"
                ),
            ),
        ],
    );
    let before = tree(&root);
    let (doc, _, _) = scan(&[root.to_str().unwrap(), "--language", "rust"]);
    let files: Vec<&str> = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(files.contains(&"app/src/lib.rs"), "{files:?}");
    assert_eq!(m.fired(), Vec::<String>::new());
    assert_eq!(tree(&root), before);
    // The flag runs both, which shows the fixture's code is reachable; the build output still
    // goes to a temporary directory.
    scan(&[
        root.to_str().unwrap(),
        "--language",
        "rust",
        "--rust-build-scripts",
    ]);
    assert_eq!(m.fired(), ["build-rs", "proc-macro"]);
    assert_eq!(tree(&root), before);
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_refuses_a_cargo_env_table() {
    assert_config_refused(
        "rust-env",
        ".cargo/config.toml",
        "[env]\nDYLD_INSERT_LIBRARIES = \"{dylib}\"\nLD_PRELOAD = \"{dylib}\"\n",
        "env",
    );
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_refuses_a_cargo_unstable_table() {
    // rust-analyzer runs some cargo commands with RUSTC_BOOTSTRAP=1, where [unstable] applies.
    assert_config_refused(
        "rust-unstable",
        ".cargo/config.toml",
        "[unstable]\nhost-config = true\ntarget-applies-to-host = false\n",
        "unstable",
    );
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_refuses_a_cargo_build_target() {
    assert_config_refused(
        "rust-build-target",
        ".cargo/config.toml",
        "[build]\ntarget = \"wasm32-unknown-unknown\"\n",
        "build.target",
    );
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_refuses_a_cargo_rustc() {
    assert_config_refused(
        "rust-rustc",
        ".cargo/config.toml",
        "[build]\nrustc = \"{script}\"\n",
        "build.rustc",
    );
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_refuses_a_cargo_rustc_wrapper() {
    assert_config_refused(
        "rust-rustc-wrapper",
        ".cargo/config.toml",
        "[build]\nrustc-wrapper = \"{script}\"\n",
        "build.rustc-wrapper",
    );
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_refuses_a_cargo_rustc_workspace_wrapper() {
    assert_config_refused(
        "rust-rustc-workspace-wrapper",
        ".cargo/config.toml",
        "[build]\nrustc-workspace-wrapper = \"{script}\"\n",
        "build.rustc-workspace-wrapper",
    );
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_refuses_a_cargo_linker_in_an_ancestor_config() {
    // `.cargo/config`, without extension, one directory above the repository.
    let host = host();
    assert_config_refused(
        "rust-linker",
        "../.cargo/config",
        &format!("[target.{host}]\nlinker = \"{{script}}\"\n"),
        &format!("target.{host}.linker"),
    );
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_refuses_a_cargo_runner() {
    assert_config_refused(
        "rust-runner",
        ".cargo/config.toml",
        "[target.'cfg(all())']\nrunner = \"{script}\"\n",
        "target.cfg(all()).runner",
    );
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_scan_drops_repository_rustflags() {
    // rust-analyzer runs `cargo rustc --print` with RUSTC_BOOTSTRAP=1, where -Z flags apply:
    // either form would load the marker into rustc without CARGO_ENCODED_RUSTFLAGS.
    let host = host();
    for (name, config) in [
        (
            "rust-build-rustflags",
            "[build]\nrustflags = [\"-Zcodegen-backend={dylib}\"]\n".to_owned(),
        ),
        (
            "rust-target-rustflags",
            format!("[target.{host}]\nrustflags = [\"-Zcodegen-backend={{dylib}}\"]\n"),
        ),
    ] {
        let m = Markers::new(name);
        let root = rust_crate(name, &[(".cargo/config.toml", &m.fill(&config))]);
        let before = tree(&root);
        for extra in [&[][..], &["--rust-build-scripts"]] {
            let (doc, _, _) =
                scan(&[&[root.to_str().unwrap(), "--language", "rust"], extra].concat());
            assert_eq!(doc["files"].as_array().unwrap().len(), 2, "{name}");
        }
        assert_eq!(m.fired(), Vec::<String>::new(), "{name}");
        assert_eq!(tree(&root), before, "{name}");
    }
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_never_downloads_or_runs_a_repository_toolchain() {
    let m = Markers::new("rust-toolchain");
    // A channel that is not installed, with rustup's download server unreachable.
    let root = rust_crate(
        "rust-toolchain",
        &[("rust-toolchain.toml", "[toolchain]\nchannel = \"1.60.0\"\n")],
    );
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_boundrail"))
        .args(["scan", root.to_str().unwrap(), "--language", "rust"])
        .env("RUSTUP_DIST_SERVER", "http://127.0.0.1:9")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("names toolchain 1.60.0, which is not installed"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("127.0.0.1"),
        "rustup tried a download: {stderr}"
    );
    // A plain `rust-toolchain` in an ancestor counts as well.
    let root = rust_crate("rust-toolchain-legacy", &[]);
    fs::write(
        root.parent().unwrap().join("rust-toolchain"),
        "nightly-2001-01-01\n",
    )
    .unwrap();
    rust_refused(
        &root,
        "names toolchain nightly-2001-01-01, which is not installed",
    );
    // A toolchain by path names programs in the repository.
    let tc = "#!/bin/sh\ntouch '{fired}/tc-'$$\n";
    let tc = tc.replace("{fired}", &m.dir.display().to_string());
    let root = rust_crate(
        "rust-toolchain-path",
        &[
            ("rust-toolchain.toml", "[toolchain]\npath = \"evil-tc\"\n"),
            ("evil-tc/bin/cargo", &tc),
            ("evil-tc/bin/rustc", &tc),
        ],
    );
    rust_refused(&root, "sets `toolchain.path`");
    assert_eq!(m.fired(), Vec::<String>::new());
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_analyzer_toml_is_ignored() {
    // `rust-analyzer scip` 1.95 reads no rust-analyzer.toml: none of these keys applies.
    let m = Markers::new("rust-ratoml");
    let ratoml = m.fill(
        "[cargo]\nextraEnv = { DYLD_INSERT_LIBRARIES = \"{dylib}\", LD_PRELOAD = \"{dylib}\" }\n\
         [cargo.buildScripts]\noverrideCommand = [\"{script}\"]\n\
         [procMacro]\nserver = \"{script}\"\n",
    );
    let root = rust_crate("rust-ratoml", &[("rust-analyzer.toml", &ratoml)]);
    let before = tree(&root);
    for extra in [&[][..], &["--rust-build-scripts"]] {
        scan(&[&[root.to_str().unwrap(), "--language", "rust"], extra].concat());
    }
    assert_eq!(m.fired(), Vec::<String>::new());
    assert_eq!(tree(&root), before);
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_refuses_a_repository_without_cargo_lock() {
    let root = rust_crate("rust-no-lock", &[]);
    fs::remove_file(root.join("Cargo.lock")).unwrap();
    let before = tree(&root);
    rust_refused(&root, "run `cargo generate-lockfile`");
    assert_eq!(tree(&root), before);
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_refuses_a_rust_project_json() {
    // rust-analyzer would prefer it to Cargo.toml and load the proc-macro library it names.
    let m = Markers::new("rust-project");
    let project = m.fill(
        "{\"crates\": [{\"root_module\": \"src/lib.rs\", \"edition\": \"2021\", \"deps\": [], \
         \"is_proc_macro\": true, \"proc_macro_dylib_path\": \"{dylib}\"}]}",
    );
    let root = rust_crate("rust-project", &[("rust-project.json", &project)]);
    rust_refused(&root, "rust-project.json");
    let root = rust_crate("rust-project-ancestor", &[]);
    fs::write(root.parent().unwrap().join(".rust-project.json"), &project).unwrap();
    rust_refused(&root, ".rust-project.json");
    assert_eq!(m.fired(), Vec::<String>::new());
}

#[test]
#[ignore = "needs rustup's active toolchain with rust-analyzer and rust-src; run with --ignored"]
fn rust_fails_on_a_degraded_load() {
    // A locked dependency missing from the cargo cache: rust-analyzer would index the
    // workspace without it.
    let root = rust_crate(
        "rust-missing-dep",
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"w\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
                 [dependencies]\nzzzz-boundrail-missing = \"1\"\n",
            ),
            (
                "Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"w\"\nversion = \"0.1.0\"\n\
                 dependencies = [\"zzzz-boundrail-missing\"]\n\n[[package]]\n\
                 name = \"zzzz-boundrail-missing\"\nversion = \"1.0.0\"\n\
                 source = \"registry+https://github.com/rust-lang/crates.io-index\"\n\
                 checksum = \"0000000000000000000000000000000000000000000000000000000000000000\"\n",
            ),
        ],
    );
    let before = tree(&root);
    rust_refused(&root, "cargo fetch --locked");
    assert_eq!(tree(&root), before);
    // A crate whose only source is outside the scanned root: nothing indexed under it.
    let root = rust_crate(
        "rust-empty",
        &[(
            "Cargo.toml",
            "[package]\nname = \"w\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
             [lib]\npath = \"../outside/lib.rs\"\n",
        )],
    );
    fs::remove_dir_all(root.join("src")).unwrap();
    write_files(
        root.parent().unwrap(),
        &[("outside/lib.rs", "pub fn f() {}\n")],
    );
    rust_refused(&root, "indexed no files");
}

#[test]
#[ignore = "needs depcruise with TypeScript beside it; run with --ignored"]
fn impact_collects_external_tests_and_resolves_barrels() {
    use std::process::Command;

    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("live-impact-barrels-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    write_files(
        &root,
        &[
            ("src/a.ts", "export const a = 1;\n"),
            ("src/b.ts", "export const b = 1;\n"),
            (
                "src/index.ts",
                "export { a } from './a';\nexport { b } from './b';\n",
            ),
            (
                "app/a.ts",
                "import { a } from '../src';\nexport const valueA = a;\n",
            ),
            (
                "app/b.ts",
                "import { b } from '../src';\nexport const valueB = b;\n",
            ),
            (
                "tests/a.spec.ts",
                "import { valueA } from '../app/a';\nconsole.log(valueA);\n",
            ),
            (
                "tests/b.spec.ts",
                "import { valueB } from '../app/b';\nconsole.log(valueB);\n",
            ),
        ],
    );
    let git = |args: &[&str]| {
        let output = Command::new(tool("BOUNDRAIL_GIT", "git"))
            .arg("-C")
            .arg(&root)
            .args([
                "-c",
                "user.name=Impact Live",
                "-c",
                "user.email=impact-live@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "initial barrel consumers"]);
    let base = git(&["rev-parse", "HEAD"]);
    fs::write(root.join("src/a.ts"), "export const a = 2;\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "change a"]);
    let head = git(&["rev-parse", "HEAD"]);
    let depcruise = fs::canonicalize(tool("BOUNDRAIL_DEPCRUISE", "depcruise")).unwrap();
    let out = boundrail(&[
        "impact",
        &format!("{base}..{head}"),
        "--root",
        root.to_str().unwrap(),
        "--depcruise",
        depcruise.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    validate(&report);
    assert_eq!(report["changed_files"], json!(["src/a.ts"]));
    assert_eq!(report["selected_tests"], json!(["tests/a.spec.ts"]));
    assert!(
        report["affected_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "app/a.ts")
    );
    assert!(
        !report["affected_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "app/b.ts")
    );
    assert!(
        report["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line
                .as_str()
                .unwrap()
                .contains("Pure re-export barrels consume no depth step"))
    );
    assert_eq!(git(&["status", "--porcelain"]), "");
    assert_eq!(git(&["rev-parse", "HEAD"]), head);
    fs::remove_dir_all(root).unwrap();
}
