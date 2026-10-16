//! Runs the real collectors. Needs depcruise 18.4.0 (with typescript 6.0.3 beside it), a
//! python with grimp 3.17 and go 1.27.1, from ARCHGRAPH_DEPCRUISE / ARCHGRAPH_PYTHON /
//! ARCHGRAPH_GO or PATH, plus node and git (ARCHGRAPH_NODE / ARCHGRAPH_GIT or PATH).

mod common;

use std::path::{Path, PathBuf};
use std::{env, fs};

use common::{archgraph, fixture, scan, scan_recorded, stable, validate};
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
    let bin = fs::canonicalize(tool("ARCHGRAPH_DEPCRUISE", "depcruise")).unwrap();
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
            let edges = live["edges"].as_array().unwrap();
            let alias = json!({ "from": "src/main.ts", "to": "src/app/shell/run.ts", "kind": "static",
                        "verdict": null, "rule": null });
            let type_only = json!({ "from": "src/app/shell/run.ts", "to": "src/core/model/types.ts",
                                    "kind": "type", "verdict": null, "rule": null });
            assert!(edges.contains(&alias) && edges.contains(&type_only));
        }
    }
}

#[test]
#[ignore = "needs depcruise 18.4.0 / grimp 3.17; run with --ignored"]
fn depcruise_path_runs_a_pinned_install_outside_the_scanned_repository() {
    // No node_modules beside the fixture and only node and git on PATH: npx could not find
    // depcruise, so a matching scan proves --depcruise ran.
    let bin = fs::canonicalize(tool("ARCHGRAPH_DEPCRUISE", "depcruise")).unwrap();
    let dirs = [tool("ARCHGRAPH_NODE", "node"), tool("ARCHGRAPH_GIT", "git")]
        .map(|p| p.parent().unwrap().to_owned());
    assert!(dirs.iter().all(|d| !d.join("depcruise").exists()));
    let dir = fixture("ts-basic");
    assert!(!dir.join("node_modules").exists());
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_archgraph"))
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
    let python = tool("ARCHGRAPH_PYTHON", "python3");
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
    let python = tool("ARCHGRAPH_PYTHON", "python3");
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
        doc["edges"],
        json!([{ "from": "src/pkg/a.py", "to": "src/pkg/b.py", "kind": "static", "verdict": null,
                 "rule": null }])
    );
}

#[test]
#[ignore = "needs go 1.27.1; run with --ignored"]
fn go_live_matches_recorded() {
    let go = tool("ARCHGRAPH_GO", "go");
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
    let go = tool("ARCHGRAPH_GO", "go");
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
    let out = archgraph(&["scan", root.to_str().unwrap(), "--go", go.to_str().unwrap()]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("go.mod requires go >= 1.99"), "{stderr}");
    assert!(stderr.contains("archgraph never downloads one"), "{stderr}");
}

#[test]
#[ignore = "needs go 1.27.1; run with --ignored"]
fn go_scan_runs_no_repository_code() {
    // Every place repository code could run from: go:generate, init and main, a test's TestMain,
    // and a go.work the go command would read (its go line would fail the scan).
    let go = tool("ARCHGRAPH_GO", "go");
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
        doc["edges"],
        json!([
            { "from": "main.go", "to": "a/a.go", "kind": "static", "verdict": null, "rule": null }
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
    let go = tool("ARCHGRAPH_GO", "go");
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
    let go = tool("ARCHGRAPH_GO", "go");
    let root = go_module(
        "go-hostile-goflags",
        "module example.com/t\n\ngo 1.21\n",
        &[("a/a.go", "package a\n".to_owned())],
    );
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_archgraph"))
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
    let go = tool("ARCHGRAPH_GO", "go");
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
        doc["edges"],
        json!([{ "from": "legacy/l.go", "to": "legacy/sub/s.go", "kind": "static", "verdict": null,
                 "rule": null }])
    );
    let (doc, _, stderr) = scan(&[root.to_str().unwrap(), "--go", go, "--exclude", "legacy/**"]);
    assert!(!stderr.contains("warning"), "{stderr}");
    assert_eq!(
        doc["files"],
        json!([{ "path": "a/a.go", "module": "a", "barrel": false }])
    );
}
