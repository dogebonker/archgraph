//! Project-local native agent hooks. Post-tool feedback never rolls back edits.
use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(clap::ValueEnum, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
    Cursor,
    Opencode,
}

#[derive(clap::ValueEnum, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Event {
    Pre,
    Post,
    Stop,
}

#[derive(Debug)]
pub struct Call {
    pub session: String,
    pub tool_id: Option<String>,
    pub files: Vec<String>,
    pub tool: String,
    pub failed: bool,
}

fn identity(value: &Value, fields: &[&str]) -> Result<String> {
    let id = fields
        .iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .context("hook payload is missing a correlation identity")?;
    ensure!(
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b)),
        "unsafe hook correlation identity"
    );
    Ok(id.to_owned())
}

fn paths(input: &Value, files: &mut BTreeSet<String>) {
    for key in ["file_path", "path", "filePath"] {
        if let Some(path) = input.get(key).and_then(Value::as_str) {
            files.insert(path.to_owned());
        }
    }
    if let Some(items) = input.get("files").and_then(Value::as_array) {
        for item in items {
            if let Some(path) = item.as_str() {
                files.insert(path.to_owned());
            } else {
                paths(item, files);
            }
        }
    }
    // Read patch protocol headers only; tool commands are never executed.
    for key in ["command", "patch", "input"] {
        if let Some(patch) = input.get(key).and_then(Value::as_str) {
            patch_paths(patch, files);
        }
    }
    if let Some(patch) = input.as_str() {
        patch_paths(patch, files);
    }
}

fn patch_paths(patch: &str, files: &mut BTreeSet<String>) {
    if !patch.lines().any(|line| line == "*** Begin Patch") {
        return;
    }
    for line in patch.lines() {
        for prefix in [
            "*** Add File: ",
            "*** Update File: ",
            "*** Delete File: ",
            "*** Move to: ",
        ] {
            if let Some(path) = line.strip_prefix(prefix) {
                files.insert(path.to_owned());
            }
        }
    }
}

pub fn decode(agent: Agent, event: Event, input: &Value) -> Result<Call> {
    ensure!(input.is_object(), "hook payload must be a JSON object");
    let (session_fields, tool_fields): (&[&str], &[&str]) = match agent {
        Agent::Claude | Agent::Codex => (&["session_id"], &["tool_use_id"]),
        Agent::Cursor => (&["conversation_id"], &["tool_use_id"]),
        Agent::Opencode => (&["sessionID"], &["callID"]),
    };
    let session = identity(input, session_fields)?;
    let tool_id = if event == Event::Stop {
        None
    } else {
        Some(identity(input, tool_fields)?)
    };
    if agent == Agent::Cursor {
        identity(input, &["generation_id"])?;
    }
    let mut files = BTreeSet::new();
    paths(
        input
            .get("tool_input")
            .or_else(|| input.get("args"))
            .unwrap_or(input),
        &mut files,
    );
    let tool = input
        .get("tool_name")
        .or_else(|| input.get("tool"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    ensure!(
        event == Event::Stop || !tool.is_empty(),
        "hook payload is missing tool name"
    );
    let output = input
        .get("tool_response")
        .or_else(|| input.get("tool_output"));
    let failed = input
        .get("failed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || input
            .get("error")
            .is_some_and(|v| !v.is_null() && v != &json!(false))
        || output.is_some_and(|o| {
            o.get("is_error").and_then(Value::as_bool) == Some(true)
                || o.get("exitCode")
                    .or_else(|| o.get("exit_code"))
                    .and_then(Value::as_i64)
                    .is_some_and(|n| n != 0)
        });
    Ok(Call {
        session,
        tool_id,
        files: files.into_iter().collect(),
        tool,
        failed,
    })
}

pub fn response(
    agent: Agent,
    event: Event,
    blocked: bool,
    message: &str,
    context: &str,
) -> Result<Value> {
    let text = [message, context]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(match agent {
        Agent::Claude | Agent::Codex => match event {
            Event::Pre if blocked => {
                json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":text}})
            }
            Event::Pre => {
                json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":text}})
            }
            Event::Post => {
                let mut result = json!({"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":text}});
                if blocked {
                    result["decision"] = json!("block");
                    result["reason"] = json!(text);
                }
                result
            }
            Event::Stop if blocked => json!({"decision":"block","reason":text}),
            Event::Stop => json!({}),
        },
        Agent::Cursor => match event {
            Event::Pre => {
                json!({"permission":if blocked {"deny"} else {"allow"},"agent_message":text,"user_message":message})
            }
            Event::Post => json!({"additional_context":text}),
            Event::Stop if blocked => json!({"followup_message":text}),
            Event::Stop => json!({}),
        },
        Agent::Opencode => json!({"blocked":blocked,"message":text}),
    })
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn safe_path(root: &Path, relative: &str) -> Result<std::path::PathBuf> {
    let mut path = root.to_path_buf();
    for part in Path::new(relative).components() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "refusing symlink in hook installation path: {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}

fn write_changed(path: &Path, contents: &str) -> Result<bool> {
    if fs::read_to_string(path).ok().as_deref() == Some(contents) {
        return Ok(false);
    }
    fs::create_dir_all(path.parent().context("missing hook parent directory")?)?;
    // A create-new sibling prevents following an attacker-controlled temporary symlink.
    let temporary = path.with_extension(format!("boundrail-{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let result = (|| -> Result<()> {
        use std::io::Write;
        match fs::metadata(path) {
            Ok(metadata) => file.set_permissions(metadata.permissions())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(true)
}

pub fn install(root: &Path, agent: Agent, executable: &Path, feedback_only: bool) -> Result<Value> {
    let strict = matches!(agent, Agent::Claude | Agent::Codex);
    ensure!(
        strict || feedback_only,
        "strict completion enforcement is unsupported for {agent:?}; use --feedback-only for a limited advisory adapter"
    );
    ensure!(
        executable.is_absolute(),
        "hook executable must be an absolute path"
    );
    let root = root.canonicalize()?;
    ensure!(root.is_dir(), "hook project root must be a directory");
    let root_text = root.to_str().context("project path must be UTF-8")?;
    let executable_text = executable
        .to_str()
        .context("executable path must be UTF-8")?;
    let name = match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Cursor => "cursor",
        Agent::Opencode => "opencode",
    };
    let command = |event: &str| {
        format!(
            "{} hook run --agent {name} --event {event} --root {}{}",
            quote(executable_text),
            quote(root_text),
            if feedback_only {
                " --feedback-only"
            } else {
                ""
            }
        )
    };
    let relative = match agent {
        Agent::Claude => ".claude/settings.json",
        Agent::Codex => ".codex/hooks.json",
        Agent::Cursor => ".cursor/hooks.json",
        Agent::Opencode => ".opencode/plugins/boundrail.ts",
    };
    let path = safe_path(&root, relative)?;
    let contents = if agent == Agent::Opencode {
        let existing = match fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        ensure!(
            existing
                .as_ref()
                .is_none_or(|s| s.starts_with("// boundrail:managed\n")),
            "refusing to overwrite an unowned OpenCode plugin"
        );
        opencode_plugin(executable_text, root_text)?
    } else {
        let mut config: Value = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .context("existing hooks configuration is not valid JSON")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
            Err(error) => return Err(error.into()),
        };
        let object = config
            .as_object_mut()
            .context("hooks configuration must be an object")?;
        if agent == Agent::Cursor {
            ensure!(
                object.get("version").is_none_or(|v| v == &json!(1)),
                "unsupported Cursor hooks version"
            );
            object.insert("version".into(), json!(1));
        }
        let hooks = object
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("hooks must be an object")?;
        let mut events = if agent == Agent::Cursor {
            vec![
                ("pre", "preToolUse"),
                ("post", "postToolUse"),
                ("stop", "stop"),
            ]
        } else {
            vec![
                ("pre", "PreToolUse"),
                ("post", "PostToolUse"),
                ("stop", "Stop"),
            ]
        };
        if agent == Agent::Claude {
            events.push(("post", "PostToolUseFailure"));
        } else if agent == Agent::Cursor {
            events.push(("post", "postToolUseFailure"));
        }
        for (event, native) in events {
            let entries = hooks
                .entry(native)
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .context("hook event must be an array")?;
            let marker = format!(" # boundrail:managed:{event}");
            let owned = |v: &Value| {
                v.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.ends_with(&marker))
            };
            if agent == Agent::Cursor {
                entries.retain(|v| !owned(v));
                entries.push(json!({"command":format!("{}{marker}", command(event))}));
            } else {
                // Remove only our handlers, retaining foreign handlers in the same group.
                entries.retain_mut(|entry| {
                    let Some(handlers) = entry.get_mut("hooks").and_then(Value::as_array_mut)
                    else {
                        return true;
                    };
                    let had_owned = handlers.iter().any(owned);
                    handlers.retain(|v| !owned(v));
                    !had_owned || !handlers.is_empty()
                });
                let mut handler = json!({"type":"command","command":format!("{}{marker}",command(event)),"timeout":10});
                if agent == Agent::Claude && event != "stop" {
                    handler["onFailure"] = json!("block");
                }
                entries.push(json!({"hooks":[handler]}));
            }
        }
        format!("{}\n", serde_json::to_string_pretty(&config)?)
    };
    let changed = write_changed(&path, &contents)?;
    Ok(
        json!({"agent":name,"written":if changed {vec![relative]} else {vec![]},"unchanged":if changed {vec![]} else {vec![relative]},"capabilities":{"pre_tool_deny":true,"pre_edit_context":strict,"completion_gate":strict && !feedback_only,"post_edit_rollback":false,"feedback_only":!strict || feedback_only},"notice":if agent == Agent::Codex {"Review and trust project-local hooks in Codex before they execute; no trust settings were changed."} else if !strict {"Feedback-only: completion is not mechanically vetoed."} else {"Project-local hooks installed; review host trust requirements."}}),
    )
}

fn opencode_plugin(executable: &str, root: &str) -> Result<String> {
    Ok(format!(
        r#"// boundrail:managed
// Feedback-only: session.idle cannot veto completion.
import {{ spawn }} from "node:child_process";
export const Boundrail = async ({{ client }}) => {{
  const run = (event, input) => new Promise((resolve, reject) => {{
    const child = spawn({}, ["hook", "run", "--agent", "opencode", "--event", event, "--root", {}, "--feedback-only"], {{ cwd: {}, stdio: ["pipe", "pipe", "pipe"], timeout: 10000 }});
    let stdout = "", stderr = "";
    child.stdout.on("data", chunk => stdout += chunk);
    child.stderr.on("data", chunk => stderr += chunk);
    child.on("error", reject);
    child.on("close", code => {{
      if (code !== 0) return reject(new Error(stderr || "boundrail hook failed"));
      try {{ resolve(JSON.parse(stdout)); }} catch (error) {{ reject(error); }}
    }});
    child.stdin.on("error", reject);
    child.stdin.end(JSON.stringify(input));
  }});
  const notify = async result => {{
    if (result.message) await client.app.log({{ body: {{ service: "boundrail", level: result.blocked ? "warn" : "info", message: result.message }} }});
  }};
  return {{
    "tool.execute.before": async (input, output) => {{
      const result = await run("pre", {{ ...input, args: output.args }});
      await notify(result);
      if (result.blocked) throw new Error(result.message);
    }},
    "tool.execute.after": async (input, output) => {{
      const result = await run("post", input);
      await notify(result);
      if (result.message) output.output += "\n" + result.message;
    }},
    event: async ({{ event }}) => {{
      if (event.type === "session.idle") await notify(await run("stop", {{ sessionID: event.properties.sessionID }}));
    }},
  }};
}};
"#,
        serde_json::to_string(executable)?,
        serde_json::to_string(root)?,
        serde_json::to_string(root)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn directory() -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "boundrail hooks {} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn decodes_native_tools_and_patch_moves() {
        for agent in [Agent::Claude, Agent::Codex] {
            let call = decode(agent, Event::Pre, &json!({"session_id":"session-1","tool_use_id":"call-2","tool_name":"apply_patch","tool_input":{"command":"*** Begin Patch\n*** Update File: old name.rs\n*** Move to: new name.rs\n*** End Patch"}})).unwrap();
            assert_eq!(call.files, ["new name.rs", "old name.rs"]);
            assert_eq!(call.tool_id.as_deref(), Some("call-2"));
        }
        let cursor = decode(Agent::Cursor, Event::Post, &json!({"conversation_id":"conv","generation_id":"gen","tool_use_id":"tool","tool_name":"Write","tool_input":{"file_path":"src/file.rs"}})).unwrap();
        assert_eq!(cursor.files, ["src/file.rs"]);
        let open = decode(Agent::Opencode, Event::Pre, &json!({"sessionID":"ses_1","callID":"call_1","tool":"edit","args":{"filePath":"src/file.rs"}})).unwrap();
        assert_eq!(open.files, cursor.files);
    }

    #[test]
    fn requires_safe_identities_but_stop_needs_no_tool() {
        assert!(decode(Agent::Claude, Event::Pre, &json!({})).is_err());
        assert!(
            decode(
                Agent::Claude,
                Event::Stop,
                &json!({"session_id":"../escape"})
            )
            .is_err()
        );
        assert!(decode(Agent::Codex, Event::Stop, &json!({"session_id":"session"})).is_ok());
        assert!(
            decode(
                Agent::Cursor,
                Event::Stop,
                &json!({"conversation_id":"session"})
            )
            .is_err()
        );
    }

    #[test]
    fn files_and_failures_are_decoded_without_executing_commands() {
        let call = decode(Agent::Codex, Event::Post, &json!({"session_id":"s","tool_use_id":"t","tool_name":"Bash","tool_input":{"files":["a",{"path":"b"}],"command":"touch NEVER_EXECUTE"},"tool_response":{"exit_code":1}})).unwrap();
        assert_eq!(call.files, ["a", "b"]);
        assert!(call.failed);
    }

    #[test]
    fn returns_host_native_decisions() {
        assert_eq!(
            response(Agent::Claude, Event::Pre, true, "BLOCK", "recipe").unwrap()["hookSpecificOutput"]
                ["permissionDecision"],
            "deny"
        );
        assert_eq!(
            response(Agent::Codex, Event::Stop, true, "BLOCK", "").unwrap()["decision"],
            "block"
        );
        assert_eq!(
            response(Agent::Cursor, Event::Post, true, "BLOCK", "").unwrap(),
            json!({"additional_context":"BLOCK"})
        );
        assert_eq!(
            response(Agent::Cursor, Event::Stop, true, "BLOCK", "").unwrap(),
            json!({"followup_message":"BLOCK"})
        );
    }

    #[test]
    fn install_is_idempotent_and_preserves_foreign_hooks() {
        for agent in [Agent::Claude, Agent::Codex, Agent::Cursor] {
            let root = directory();
            let relative = match agent {
                Agent::Claude => ".claude/settings.json",
                Agent::Codex => ".codex/hooks.json",
                _ => ".cursor/hooks.json",
            };
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let event = if agent == Agent::Cursor {
                "preToolUse"
            } else {
                "PreToolUse"
            };
            let foreign = if agent == Agent::Cursor {
                json!({"command":"foreign command"})
            } else {
                json!({"matcher":"Edit","hooks":[{"type":"command","command":"foreign command"}]})
            };
            fs::write(
                &path,
                serde_json::to_vec(
                    &json!({"other":{"keep":true},"hooks":{event:[foreign.clone()]}}),
                )
                .unwrap(),
            )
            .unwrap();
            let first = install(&root, agent, Path::new("/opt/my tools/arch'graph"), true).unwrap();
            assert_eq!(first["written"], json!([relative]));
            let before = fs::read(&path).unwrap();
            let second =
                install(&root, agent, Path::new("/opt/my tools/arch'graph"), true).unwrap();
            assert_eq!(second["unchanged"], json!([relative]));
            assert_eq!(fs::read(&path).unwrap(), before);
            let saved: Value = serde_json::from_slice(&before).unwrap();
            if agent == Agent::Claude {
                assert_eq!(
                    saved["hooks"]["PreToolUse"][1]["hooks"][0]["onFailure"],
                    "block"
                );
                assert!(
                    saved["hooks"]["PostToolUseFailure"][0]["hooks"][0]["command"]
                        .as_str()
                        .unwrap()
                        .contains("--event post")
                );
            }
            assert_eq!(saved["other"], json!({"keep":true}));
            assert_eq!(saved["hooks"][event][0], foreign);
            let text = String::from_utf8(before).unwrap();
            assert!(text.contains("--feedback-only"));
            assert!(text.contains("--root '"));
            assert!(text.contains("arch'\\\\''graph"));
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn managed_handlers_retarget_the_current_executable() {
        for (agent, relative) in [
            (Agent::Claude, ".claude/settings.json"),
            (Agent::Codex, ".codex/hooks.json"),
            (Agent::Cursor, ".cursor/hooks.json"),
            (Agent::Opencode, ".opencode/plugins/boundrail.ts"),
        ] {
            let root = directory();
            install(&root, agent, Path::new("/previous/boundrail"), true).unwrap();
            let path = root.join(relative);
            let updated = install(&root, agent, Path::new("/current/boundrail"), true).unwrap();
            assert_eq!(updated["written"], json!([relative]));
            let text = fs::read_to_string(&path).unwrap();
            assert!(text.contains("/current/boundrail"));
            assert!(!text.contains("/previous/boundrail"));
            assert!(text.contains("boundrail:managed"));
            assert!(!root.join(".opencode/plugins/archgraph.ts").exists());
            let repeated = install(&root, agent, Path::new("/current/boundrail"), true).unwrap();
            assert_eq!(repeated["unchanged"], json!([relative]));
            assert_eq!(fs::read_to_string(path).unwrap(), text);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn reinstall_preserves_private_settings_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = directory();
        let path = root.join(".claude/settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{\"env\":{\"PRIVATE\":\"preserve\"}}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        install(&root, Agent::Claude, Path::new("/opt/boundrail"), false).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn quoted_command_preserves_executable_and_project_arguments() {
        use std::os::unix::fs::PermissionsExt;
        let root = directory();
        let executable = root.join("arch'graph executable");
        fs::write(&executable, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        install(&root, Agent::Codex, &executable, true).unwrap();
        let config: Value =
            serde_json::from_slice(&fs::read(root.join(".codex/hooks.json")).unwrap()).unwrap();
        let command = config["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .unwrap();
        assert!(output.status.success());
        let args = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            [
                "hook",
                "run",
                "--agent",
                "codex",
                "--event",
                "pre",
                "--root",
                root.canonicalize().unwrap().to_str().unwrap(),
                "--feedback-only"
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsupported_strict_installs_write_nothing() {
        for agent in [Agent::Cursor, Agent::Opencode] {
            let root = directory();
            assert!(install(&root, agent, Path::new("/bin/boundrail"), false).is_err());
            assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn opencode_plugin_has_native_events_and_no_shell_interpolation() {
        let root = directory();
        let first = install(
            &root,
            Agent::Opencode,
            Path::new("/path with spaces/boundrail"),
            true,
        )
        .unwrap();
        assert_eq!(first["capabilities"]["completion_gate"], false);
        let plugin = fs::read_to_string(root.join(".opencode/plugins/boundrail.ts")).unwrap();
        assert!(plugin.contains("tool.execute.before"));
        assert!(plugin.contains("tool.execute.after"));
        assert!(plugin.contains("session.idle"));
        assert!(plugin.contains("spawn(\"/path with spaces/boundrail\""));
        assert_eq!(
            install(
                &root,
                Agent::Opencode,
                Path::new("/path with spaces/boundrail"),
                true
            )
            .unwrap()["written"],
            json!([])
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_invalid_config_without_writes() {
        let root = directory();
        fs::create_dir(root.join(".codex")).unwrap();
        fs::write(root.join(".codex/hooks.json"), "invalid").unwrap();
        assert!(install(&root, Agent::Codex, Path::new("/bin/boundrail"), false).is_err());
        assert_eq!(
            fs::read_to_string(root.join(".codex/hooks.json")).unwrap(),
            "invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_config_and_parent() {
        use std::os::unix::fs::symlink;
        let root = directory();
        let outside = directory();
        symlink(&outside, root.join(".codex")).unwrap();
        assert!(install(&root, Agent::Codex, Path::new("/bin/boundrail"), false).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        fs::remove_file(root.join(".codex")).unwrap();
        fs::create_dir(root.join(".codex")).unwrap();
        fs::write(outside.join("hooks.json"), "{}").unwrap();
        symlink(outside.join("hooks.json"), root.join(".codex/hooks.json")).unwrap();
        assert!(install(&root, Agent::Codex, Path::new("/bin/boundrail"), false).is_err());
        assert_eq!(
            fs::read_to_string(outside.join("hooks.json")).unwrap(),
            "{}"
        );
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
