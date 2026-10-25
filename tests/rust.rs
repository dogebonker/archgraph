mod common;

use std::path::Path;

use boundrail::collect::rust::{
    Features, cargo_config_refusal, config, config_error, degraded_load, toolchain_channel,
};
use common::{scan, write_tmp};
use serde_json::json;

#[test]
fn cargo_config_keys_that_run_programs_or_change_the_environment_are_refused() {
    for (text, key) in [
        ("[env]\nDYLD_INSERT_LIBRARIES = \"/x.dylib\"\n", "env"),
        ("[env]\n", "env"),
        // A dotted key names its refused table, as a table header does.
        ("env.LD_PRELOAD = \"/x.so\"\n", "env"),
        (
            "env = { LD_PRELOAD = { value = \"x\", force = true } }\n",
            "env",
        ),
        ("[unstable]\nbuild-std = [\"std\"]\n", "unstable"),
        (
            "[build]\ntarget = \"x86_64-unknown-linux-gnu\"\n",
            "build.target",
        ),
        ("build = { target = [\"a\", \"b\"] }\n", "build.target"),
        ("[build]\nrustc = \"/evil\"\n", "build.rustc"),
        (
            "[build]\nrustc-wrapper = \"/evil\"\n",
            "build.rustc-wrapper",
        ),
        (
            "[build]\nrustc_wrapper = \"/evil\"\n",
            "build.rustc_wrapper",
        ),
        (
            "[build]\nrustc-workspace-wrapper = \"/evil\"\n",
            "build.rustc-workspace-wrapper",
        ),
        ("[build]\nrustdoc = \"/evil\"\n", "build.rustdoc"),
        // Escapes in a quoted key, spaces around dots, a dotted key under a table.
        ("[build]\n\"\\u0072ustc\" = \"/evil\"\n", "build.rustc"),
        (
            "[ build ]\n'rustc-wrapper' = \"/evil\"\n",
            "build.rustc-wrapper",
        ),
        ("[\"build\"]\nrustc = \"/evil\"\n", "build.rustc"),
        ("build . rustc = \"/evil\"\n", "build.rustc"),
        (
            "[target.x86_64-unknown-linux-gnu]\nlinker = \"/evil\"\n",
            "target.x86_64-unknown-linux-gnu.linker",
        ),
        (
            "[target.'cfg(unix)']\nrunner = \"/evil\"\n",
            "target.cfg(unix).runner",
        ),
        (
            "target.\"aarch64-apple-darwin\".linker = \"/evil\"\n",
            "target.aarch64-apple-darwin.linker",
        ),
        (
            "[target]\naarch64-apple-darwin = { runner = [\"/evil\"] }\n",
            "target.aarch64-apple-darwin.runner",
        ),
        // A `links` override names libraries for a build script's package.
        (
            "[target.x86_64-unknown-linux-gnu.foo]\nrustc-link-lib = [\"evil\"]\n",
            "target.x86_64-unknown-linux-gnu.foo",
        ),
        ("[host]\nlinker = \"/evil\"\n", "host.linker"),
        (
            "[registry]\ncredential-provider = \"/evil\"\n",
            "registry.credential-provider",
        ),
        ("[doc]\nbrowser = \"/evil\"\n", "doc.browser"),
        ("include = \"other.toml\"\n", "include"),
    ] {
        let refusal = cargo_config_refusal(text).expect_err(text);
        assert!(refusal.contains(&format!("`{key}`")), "{text}: {refusal}");
    }
}

#[test]
fn cargo_config_keys_that_run_nothing_are_allowed() {
    // ripgrep's .cargo/config.toml: target rustflags, which CARGO_ENCODED_RUSTFLAGS empties.
    let ripgrep = "# On Windows MSVC, statically link the C runtime.\n\
        [target.x86_64-pc-windows-msvc]\n\
        rustflags = [\"-C\", \"target-feature=+crt-static\"]\n\
        [target.x86_64-unknown-linux-musl]\n\
        rustflags = [\n  \"-C\", \"target-feature=+crt-static\",\n  \"-C\", \"link-self-contained=yes\", # trailing\n]\n";
    for text in [
        ripgrep,
        "",
        "[build]\nrustflags = [\"-Zcodegen-backend=/evil.dylib\"]\njobs = 4\n",
        "[target.'cfg(unix)']\nrustflags = [\"-Cdebuginfo=0\"]\n",
        "[alias]\nb = \"build\"\nxtask = \"run --package xtask --\"\n",
        "[net]\noffline = true\ngit-fetch-with-cli = true\n",
        // Text that only looks like a refused key: in strings and comments.
        "[alias]\nx = \"\"\"\n[env]\nrustc = 1\n\"\"\"\ny = '''\n[unstable]'''\n",
        "# [env]\n[alias] # build.rustc = \"x\"\nz = \"[env]\" # env.X = 1\n",
        "[term]\nverbose = false\ncolor = 'auto'\n[profile.release]\nlto = true\ndebug = 1\nopt-level = 3\n",
        "[patch.crates-io]\nfoo = { path = \"../foo\" }\n[source.crates-io]\nreplace-with = \"vendored\"\n",
        "[[bin]]\nname = \"x\"\n[http]\ntimeout = 30\nwhen = 1979-05-27T07:32:00Z\n",
    ] {
        assert_eq!(cargo_config_refusal(text), Ok(()), "{text}");
    }
}

#[test]
fn a_cargo_config_that_cannot_be_read_is_refused() {
    for text in [
        "[build\nrustc = \"x\"\n",
        "[build]\nrustc = \"x\n",
        "[build]\n\"\\q\" = 1\n",
        "[build]\nrustc\n",
        "[build]\nx = \"\"\"never closed\n",
        "[build]\nx = [1, 2\n",
        "[build]\nx = { a = 1\n",
    ] {
        let refusal = cargo_config_refusal(text).expect_err(text);
        assert!(refusal.contains("cannot read"), "{text}: {refusal}");
    }
}

#[test]
fn toolchain_files_name_a_channel_or_are_refused() {
    for (name, text, want) in [
        (
            "rust-toolchain.toml",
            "[toolchain]\nchannel = \"1.60.0\"\n",
            Some("1.60.0"),
        ),
        (
            "rust-toolchain.toml",
            "[toolchain]\ncomponents = [\"rustfmt\"]\nprofile = \"minimal\"\n",
            None,
        ),
        (
            "rust-toolchain",
            "nightly-2020-01-01\n",
            Some("nightly-2020-01-01"),
        ),
        (
            "rust-toolchain",
            "[toolchain]\nchannel = 'stable'\n",
            Some("stable"),
        ),
    ] {
        assert_eq!(
            toolchain_channel(name, text),
            Ok(want.map(str::to_owned)),
            "{text}"
        );
    }
    let refusal =
        toolchain_channel("rust-toolchain.toml", "[toolchain]\npath = \"evil-tc\"\n").unwrap_err();
    assert!(refusal.contains("`toolchain.path`"), "{refusal}");
}

const SYSROOT: &str = "/home/u/.rustup/toolchains/stable-x/lib/rustlib/src/rust/library";

fn fallback(package: &str) -> String {
    format!(
        "rust-analyzer: Loading cargo metadata: failed exit status: 101\n\
         2026-10-16T20:38:58.70002+01:00  WARN `cargo metadata` failed and returning succeeded \
         result with `--no-deps` error=`cargo metadata` exited with an error: error: no matching \
         package named `x` found\nlocation searched: crates.io index\nrequired by package \
         `{package}`\nAs a reminder, you're using offline mode (--offline)\n\n\
         rust-analyzer: Loading cargo metadata: finished\n"
    )
}

#[test]
fn a_dependency_fallback_outside_the_sysroot_is_a_degraded_load() {
    // Offline, the sysroot's own dependencies are never there: that fallback is expected.
    let sysroot = fallback(&format!("std v0.0.0 ({SYSROOT}/std)"));
    let stderr = format!("Generating SCIP start...\n{sysroot}Generating SCIP finished 1.6s\n");
    assert_eq!(degraded_load(&stderr, SYSROOT), None);
    let workspace = fallback("d v0.1.0 (/repo)");
    let stderr = format!("{workspace}{sysroot}");
    let degraded = degraded_load(&stderr, SYSROOT).unwrap();
    assert!(
        degraded.contains("required by package `d v0.1.0 (/repo)`"),
        "{degraded}"
    );
    assert!(!degraded.contains("std v0.0.0"), "{degraded}");
}

#[test]
fn only_a_non_empty_config_error_sink_is_an_error() {
    let ok = "2026-10-16T20:37:29.840443+01:00 ERROR Config Error(s) error_sink=ConfigErrors([])\n";
    assert_eq!(config_error(ok), None);
    let bad = "2026-10-16T20:37:29Z ERROR Config Error(s) error_sink=ConfigErrors([Json { \
               config_key: \"cargo/features\" }])\nrust-analyzer: Loading\n";
    assert!(config_error(bad).unwrap().contains("cargo/features"));
}

#[test]
fn the_rust_analyzer_config_overrides_build_scripts_and_keeps_cfg_test_off() {
    let target = Path::new("/tmp/t/target");
    let tru = Path::new("/usr/bin/true");
    assert_eq!(
        config(Some(tru), target, Features::All),
        json!({ "cargo": { "buildScripts": { "overrideCommand": ["/usr/bin/true"] },
                           "targetDir": "/tmp/t/target", "features": "all" },
                // cfg(not(test)) items stay defined; test code is left out lexically.
                "cfg": { "setTest": false } })
    );
    assert_eq!(
        config(None, target, Features::Default),
        json!({ "cargo": { "targetDir": "/tmp/t/target", "features": [] },
                "cfg": { "setTest": false } })
    );
}

#[test]
fn rust_collector_input_lists_the_indexed_files() {
    let input = json!({
        "rust_analyzer": "1.95.0 (59807616 2026-04-14)",
        "toolchain": "stable-aarch64-apple-darwin",
        "features": "all",
        "build_scripts": false,
        "packages": [{ "name": "w", "dir": ".",
                       "targets": [{ "name": "w", "kind": ["lib"], "src_path": "src/lib.rs" }] }],
        "documents": [
            { "path": "src/a.rs", "occurrences": [], "symbols": [] },
            { "path": "src/lib.rs", "occurrences": [], "symbols": [] },
            { "path": "tests/t.rs", "occurrences": [], "symbols": [] }
        ]
    });
    let input = write_tmp("rust-input/rust.json", &input.to_string());
    let root = Path::new(&input).parent().unwrap().to_str().unwrap();
    let (doc, _, _) = scan(&[root, "--language", "rust", "--collector-input", &input]);
    assert_eq!(doc["source"]["language"], "rust");
    assert_eq!(
        doc["source"]["collector"],
        json!({ "name": "rust-analyzer", "version": "1.95.0 (59807616 2026-04-14)", "cruised": 3 })
    );
    let files: Vec<&str> = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(files, ["src/a.rs", "src/lib.rs"]);
}

#[test]
fn rust_edges_resolve_workspace_references_and_trait_implementations() {
    let occurrence = |symbol: &str, roles| {
        json!({
            "range": [0, 0, 1], "symbol": symbol, "roles": roles
        })
    };
    let input = json!({
        "rust_analyzer": "test", "toolchain": "stable", "features": "default",
        "build_scripts": false, "packages": [],
        "documents": [
            { "path": "src/api.rs", "occurrences": [occurrence("Trait", 1)], "symbols": [] },
            { "path": "src/model.rs", "occurrences": [occurrence("Model", 1),
                occurrence("crate/", 0), occurrence("parent/", 0), occurrence("parent/model/", 1),
                occurrence("entry", 0), occurrence("sibling/", 0)],
              "symbols": [{ "symbol": "Model", "kind": 0, "implements": ["Trait"] },
                          { "symbol": "parent/model/", "kind": 29 }] },
            { "path": "src/lib.rs", "occurrences": [
                occurrence("crate/", 1), occurrence("entry", 1), occurrence("Model", 0), occurrence("Model", 0),
                occurrence("entry", 0), occurrence("external", 0), occurrence("local 0", 0)
            ], "symbols": [{ "symbol": "crate/", "kind": 29, "signature": "extern crate w" }] },
            { "path": "src/parent.rs", "occurrences": [occurrence("parent/", 1)],
              "symbols": [{ "symbol": "parent/", "kind": 29 }] },
            { "path": "src/sibling.rs", "occurrences": [occurrence("sibling/", 1)],
              "symbols": [{ "symbol": "sibling/", "kind": 29 }] },
            { "path": "src/other.rs", "occurrences": [occurrence("local 0", 1)], "symbols": [] }
        ]
    });
    let graph = boundrail::collect::rust::parse(&serde_json::to_vec(&input).unwrap()).unwrap();
    let edges: Vec<_> = graph
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str(), e.kind))
        .collect();
    assert_eq!(
        edges,
        [
            (
                "src/lib.rs",
                "src/model.rs",
                boundrail::model::EdgeKind::Static
            ),
            (
                "src/model.rs",
                "src/api.rs",
                boundrail::model::EdgeKind::Static
            ),
            (
                "src/model.rs",
                "src/lib.rs",
                boundrail::model::EdgeKind::Static
            ),
            (
                "src/model.rs",
                "src/sibling.rs",
                boundrail::model::EdgeKind::Static
            ),
        ]
    );
}
