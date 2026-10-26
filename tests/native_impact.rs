mod common;

#[cfg(unix)]
#[test]
fn native_impact_and_saved_native_graph_never_run_node_or_prepare_unused_collectors() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;

    let file = common::write_tmp(
        "native-impact-project/src/model.ts",
        "export class Model {}\n",
    );
    let root = Path::new(&file).ancestors().nth(2).unwrap();
    common::write_tmp(
        "native-impact-project/src/index.ts",
        "export { Model } from './model';\n",
    );
    common::write_tmp(
        "native-impact-project/src/main.ts",
        "import { Model } from './index';\nexport const model = new Model();\n",
    );
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=archgraph",
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
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "first"]);
    fs::write(&file, "export class Model { value = 1 }\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "change model"]);

    let node = common::write_tmp(
        "native-impact-tools/node",
        "#!/bin/sh\nprintf called > \"$NATIVE_NODE_MARKER\"\nexit 91\n",
    );
    fs::set_permissions(&node, fs::Permissions::from_mode(0o755)).unwrap();
    let marker = Path::new(&node).with_file_name("called");
    let path = std::env::join_paths(
        std::iter::once(Path::new(&node).parent().unwrap().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let graph = common::write_tmp("native-impact-graphs/head.json", "");
    let output = common::archgraph(&[
        "scan",
        root.to_str().unwrap(),
        "--engine",
        "native",
        "--language",
        "typescript",
        "-o",
        &graph,
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    for saved in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_boundrail"));
        command.args([
            "impact",
            "HEAD~1..HEAD",
            "--root",
            root.to_str().unwrap(),
            "--depcruise",
            "/nonexistent/unused-depcruise",
            "--rust-analyzer",
            "/nonexistent/unused-analyzer",
            "--python",
            "/nonexistent/unused-python",
            "--java",
            "/nonexistent/unused-java",
            "--go",
            "/nonexistent/unused-go",
        ]);
        if saved {
            command.args(["--graph", &graph]);
        } else {
            command.args(["--engine", "native", "--language", "typescript"]);
        }
        let output = command
            .env("PATH", &path)
            .env("NATIVE_NODE_MARKER", &marker)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "saved={saved}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!marker.exists(), "native impact invoked Node");
    }
}
