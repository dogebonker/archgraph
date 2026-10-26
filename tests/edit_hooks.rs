use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "boundrail-native-hook-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("src/domain")).unwrap();
        fs::create_dir_all(root.join("src/adapters")).unwrap();
        fs::write(root.join("package.json"), "{\"type\":\"module\"}").unwrap();
        fs::write(root.join("src/domain/a.ts"), "export const a = 1;\n").unwrap();
        fs::write(root.join("src/adapters/b.ts"), "export const b = 2;\n").unwrap();
        fs::write(root.join("layout.yaml"), "version: 1\nforbid:\n  - {id: domain-no-adapters, from: 'src/domain/**', to: 'src/adapters/**', reason: 'Keep domain independent'}\n").unwrap();
        Self(root)
    }
    fn command(&self, args: &[&str], input: Option<Value>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_boundrail"));
        command
            .args(["--no-daemon"])
            .args(args)
            .current_dir(&self.0)
            .env_remove("BOUNDRAIL_BYPASS");
        if let Some(input) = input {
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&serde_json::to_vec(&input).unwrap())
                .unwrap();
            child.wait_with_output().unwrap()
        } else {
            command.output().unwrap()
        }
    }
    fn json(&self, args: &[&str], input: Option<Value>) -> Value {
        let output = self.command(args, input);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn native(&self, agent: &str, event: &str, id: &str, failed: bool) -> Value {
        self.json(&["hook", "run", "--agent", agent, "--event", event], Some(json!({"session_id":"fixture","tool_use_id":id,"tool_name":"Edit","tool_input":{"file_path":"src/domain/a.ts"},"hook_event_name":if failed {"PostToolUseFailure"} else {"PostToolUse"},"error":if failed {json!("partial write")} else {Value::Null}})))
    }
    fn violate(&self) {
        fs::write(
            self.0.join("src/domain/a.ts"),
            "import { b } from '../adapters/b'; export const a = b;\n",
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn native_protocol_blocks_then_allows_repair_for_claude_and_codex() {
    for agent in ["claude", "codex"] {
        let fixture = Fixture::new();
        fixture.json(&["hook", "install", "--agent", agent], None);
        let pre = fixture.native(agent, "pre", "edit", false);
        assert!(
            pre["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .len()
                <= 400
        );
        fixture.violate();
        let post = fixture.native(agent, "post", "edit", false);
        assert_eq!(post["decision"], "block");
        assert!(
            post["reason"]
                .as_str()
                .unwrap()
                .contains("domain-no-adapters")
        );
        assert_eq!(
            fixture.native(agent, "stop", "", false)["decision"],
            "block"
        );
        fixture.native(agent, "pre", "repair", false);
        fs::write(fixture.0.join("src/domain/a.ts"), "export const a = 1;\n").unwrap();
        assert!(
            fixture
                .native(agent, "post", "repair", false)
                .get("decision")
                .is_none()
        );
        assert_eq!(fixture.native(agent, "stop", "", false), json!({}));
        let fires = fixture.json(&["fires", "--top"], None);
        assert_eq!(fires["total"], 2);
    }
}

#[test]
fn failed_tool_checks_partial_write_and_retains_stop_gate() {
    let fixture = Fixture::new();
    fixture.json(&["hook", "install", "--agent", "claude"], None);
    fixture.native("claude", "pre", "partial", false);
    fixture.violate();
    let post = fixture.native("claude", "post", "partial", true);
    assert_eq!(
        post["hookSpecificOutput"]["hookEventName"],
        "PostToolUseFailure"
    );
    assert!(
        post["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .starts_with("BLOCK\n")
    );
    assert!(post.get("decision").is_none());
    assert_eq!(
        fixture.native("claude", "stop", "", false)["decision"],
        "block"
    );
}

#[test]
fn mcp_snapshot_uses_pre_edit_baseline_without_git_and_does_not_consume_it() {
    let fixture = Fixture::new();
    let capture = fixture.json(
        &[
            "hook",
            "capture",
            "--agent",
            "codex",
            "--files",
            "src/domain/a.ts",
        ],
        None,
    );
    fixture.violate();
    let args = json!({"snapshot":capture["snapshot"]}).to_string();
    let result = fixture.json(
        &["mcp-call", "--tool", "check_diff", "--arguments", &args],
        None,
    );
    assert_eq!(result["verdict"], "BLOCK");
    assert_eq!(result["new_forbidden"].as_array().unwrap().len(), 1);
    assert!(!result["violations"].as_array().unwrap().is_empty());
    assert!(!fixture.0.join(".boundrail/fires.log").exists());
    let checked = fixture.command(
        &[
            "hook",
            "check",
            "--baseline",
            capture["snapshot"].as_str().unwrap(),
            "--brief",
        ],
        None,
    );
    assert_eq!(checked.status.code(), Some(1));
    assert!(checked.stdout.starts_with(b"BLOCK\n{"));
}

#[test]
fn invalid_native_payload_returns_deny_instead_of_clean() {
    let fixture = Fixture::new();
    fixture.json(&["hook", "install", "--agent", "claude"], None);
    let result = fixture.json(
        &["hook", "run", "--agent", "claude", "--event", "pre"],
        Some(json!({})),
    );
    assert_eq!(result["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(
        result["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .starts_with("UNAVAILABLE\n")
    );
}

#[test]
fn missing_snapshot_has_an_unavailable_brief_verdict() {
    let fixture = Fixture::new();
    let output = fixture.command(
        &["hook", "check", "--baseline", &"a".repeat(64), "--brief"],
        None,
    );
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8(output.stdout).unwrap();
    let (verdict, body) = text.split_once('\n').unwrap();
    assert_eq!(verdict, "UNAVAILABLE");
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap()["verdict"],
        "UNAVAILABLE"
    );
}

#[test]
fn stop_detected_debt_survives_the_next_edit_and_cancelled_tools_can_finish() {
    let fixture = Fixture::new();
    fixture.json(&["hook", "install", "--agent", "claude"], None);
    fixture.native("claude", "pre", "cancelled", false);
    // A denied/cancelled tool can omit its post event; Stop still checks the same baseline.
    assert_eq!(fixture.native("claude", "stop", "", false), json!({}));
    fixture.native("claude", "pre", "clean", false);
    fixture.native("claude", "post", "clean", false);
    fixture.violate();
    assert_eq!(
        fixture.native("claude", "stop", "", false)["decision"],
        "block"
    );
    fixture.native("claude", "pre", "unchanged-repair", false);
    assert_eq!(
        fixture.native("claude", "post", "unchanged-repair", false)["decision"],
        "block"
    );
}

#[test]
fn read_tools_allow_external_paths_without_taking_the_edit_lease() {
    let fixture = Fixture::new();
    fixture.json(&["hook", "install", "--agent", "claude"], None);
    fixture.native("claude", "pre", "edit", false);
    for event in ["pre", "post"] {
        let result = fixture.json(&["hook", "run", "--agent", "claude", "--event", event], Some(json!({"session_id":"fixture", "tool_use_id":"read", "tool_name":"Read", "tool_input":{"file_path":"/outside/skills/SKILL.md"}})));
        assert!(result["hookSpecificOutput"]["permissionDecision"].is_null());
        assert!(result.get("decision").is_none());
    }
    let post = fixture.native("claude", "post", "edit", false);
    assert!(post.get("decision").is_none(), "{post}");
    assert!(
        post["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .starts_with("PASS\n")
    );
}

#[test]
fn default_layout_created_after_install_is_adopted_at_the_next_clean_pre() {
    for agent in ["claude", "codex"] {
        let fixture = Fixture::new();
        let layout = fs::read(fixture.0.join("layout.yaml")).unwrap();
        fs::remove_file(fixture.0.join("layout.yaml")).unwrap();
        fixture.json(&["hook", "install", "--agent", agent], None);
        fixture.native(agent, "pre", "initial", false);
        assert_ne!(
            fixture.native(agent, "post", "initial", false)["decision"],
            "block"
        );
        fs::write(fixture.0.join("layout.yaml"), layout).unwrap();
        let pre = fixture.native(agent, "pre", "with-layout", false);
        assert_ne!(pre["hookSpecificOutput"]["permissionDecision"], "deny");
        fixture.violate();
        let post = fixture.native(agent, "post", "with-layout", false);
        assert_eq!(post["decision"], "block");
        assert!(
            post["reason"]
                .as_str()
                .unwrap()
                .contains("domain-no-adapters")
        );
    }
}

#[test]
fn first_default_layout_created_during_an_edit_makes_post_and_stop_unavailable() {
    for agent in ["claude", "codex"] {
        let fixture = Fixture::new();
        let layout = fs::read(fixture.0.join("layout.yaml")).unwrap();
        fs::remove_file(fixture.0.join("layout.yaml")).unwrap();
        fixture.json(&["hook", "install", "--agent", agent], None);
        fixture.native(agent, "pre", "pending", false);
        fs::write(fixture.0.join("layout.yaml"), layout).unwrap();
        for event in ["post", "stop"] {
            let response = fixture.native(agent, event, "pending", false);
            assert_eq!(response["decision"], "block");
            assert!(
                response["reason"]
                    .as_str()
                    .unwrap()
                    .starts_with("UNAVAILABLE\n")
            );
        }
        let repair = fixture.native(agent, "pre", "repair", false);
        assert_eq!(repair["hookSpecificOutput"]["permissionDecision"], "deny");
    }
}
