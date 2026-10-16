//! Runs the real collectors. Needs depcruise 18.4.0 (with typescript 6.0.3 beside it) and a
//! python with grimp 3.17, from ARCHGRAPH_DEPCRUISE / ARCHGRAPH_PYTHON or PATH, plus node and git
//! (ARCHGRAPH_NODE / ARCHGRAPH_GIT or PATH).

mod common;

use std::path::{Path, PathBuf};
use std::{env, fs};

use common::{fixture, scan, scan_recorded, stable, validate};
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
