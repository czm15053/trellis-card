use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, Document};

/* Claude Code / Codex 的 hook 事件（PascalCase，settings.json/hooks.json 嵌套结构）。 */
const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Stop",
];

/* Cursor 的 hook 事件（camelCase 小写开头，hooks.json 扁平结构）。
只采集核心事件集；Cursor 无 PermissionRequest/PostToolUse 等事件。 */
const CURSOR_EVENTS: &[&str] = &[
    "sessionStart",
    "preToolUse",
    "beforeShellExecution",
    "stop",
    "sessionEnd",
];

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub agent: String,
    pub installed: bool,
    pub config_exists: bool,
    pub config_path: String,
}

fn is_owned_command(command: &str) -> bool {
    let stripped = command.replace(['"', '\''], "");
    /* 定位 ` hook ` 参数，取它之前的整段作为可执行文件路径（含空格路径）。
    无 hook 子命令的无关命令直接排除。 */
    let Some(idx) = stripped.find(" hook ") else {
        return false;
    };
    let exe_path = &stripped[..idx];
    let exe_path = exe_path.trim_end_matches(".exe");
    /* basename（去掉目录分隔符）后忽略大小写比较。
    Windows 下 exe 可能是 trellis-card.exe 或 productName 重命名的 Trellis-Card.exe。 */
    let name = exe_path.rsplit(['/', '\\']).next().unwrap_or_default();
    name.eq_ignore_ascii_case("trellis-card")
}

fn remove_owned(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        if let Some(hooks) = object.get_mut("hooks") {
            remove_owned(hooks);
        }
    }
    if let Some(array) = value.as_array_mut() {
        array.retain(|item| {
            let owned = item
                .get("command")
                .and_then(Value::as_str)
                .map(is_owned_command)
                .unwrap_or(false);
            if !owned {
                let mut item = item.clone();
                remove_owned(&mut item);
                true
            } else {
                false
            }
        });
        for item in array {
            remove_owned(item);
        }
    }
}

fn contains_owned(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object
                .get("command")
                .and_then(Value::as_str)
                .map(is_owned_command)
                .unwrap_or(false)
                || object.values().any(contains_owned)
        }
        Value::Array(items) => items.iter().any(contains_owned),
        _ => false,
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temp = path.with_extension("tmp");
    fs::write(&temp, data).map_err(|error| error.to_string())?;
    fs::rename(&temp, path).map_err(|error| error.to_string())
}

fn load_json(path: &Path) -> Result<(Value, bool), String> {
    match fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data)
            .map(|config| (config, true))
            .map_err(|error| format!("invalid JSON in {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((Value::Object(Map::new()), false))
        }
        Err(error) => Err(error.to_string()),
    }
}

fn load_toml(path: &Path) -> Result<(Document, bool), String> {
    match fs::read_to_string(path) {
        Ok(data) => data
            .parse::<Document>()
            .map(|config| (config, true))
            .map_err(|error| format!("invalid TOML in {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((Document::new(), false)),
        Err(error) => Err(error.to_string()),
    }
}

fn merge_codex_features(config: &mut Document) -> Result<(), String> {
    let features = config.entry("features").or_insert_with(toml_edit::table);
    if let Some(table) = features.as_table_mut() {
        table["hooks"] = value(true);
        return Ok(());
    }
    if let Some(table) = features.as_inline_table_mut() {
        table.insert("hooks", true.into());
        return Ok(());
    }
    Err("config.toml features must be a table".to_string())
}

/* Cursor hooks.json 的 preToolUse matcher：观察所有 Agent 工具调用（含 MCP）。
Cursor 的 matcher 对 tool_name 做大小写敏感匹配，覆盖 Claude 风格的 Shell/Read/Write/Grep/
Delete/Task/Edit 即可。 */
const CURSOR_PRE_TOOL_MATCHER: &str = "Shell|Read|Write|Grep|Delete|Task|Edit|MCP";

/* 合并 Cursor 的 hooks.json（扁平结构，区别于 Claude/Codex 的嵌套结构）。
Cursor 顶层是 { "version": 1, "hooks": { "<event>": [ { "command": "...", ... } ] } }，
hooks 数组元素直接是 { command, matcher?, timeout }，无 Claude 的 { matcher, hooks: [...] } 嵌套。
安装：对每个事件追加自有 command（已存在则跳过，幂等）；保留其他 hook（Trellis 生成的
inject-*.py、用户自定义）不动。移除：只删除自有 command，保留 version 与其他 hook。 */
fn merge_cursor_hooks(config: &mut Value, command: &str, uninstall: bool) -> Result<(), String> {
    let root = config
        .as_object_mut()
        .ok_or_else(|| "hooks config root must be a JSON object".to_string())?;
    if let Some(version) = root.get("version") {
        if version.as_u64() != Some(1) {
            return Err("unsupported cursor hooks version".to_string());
        }
    } else {
        root.insert("version".into(), json!(1));
    }
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| "hooks config hooks must be an object".to_string())?;
    if uninstall {
        for value in hooks.values_mut() {
            remove_owned(value);
        }
        /* 清理后删除空数组，但保留 hooks 对象本身（Cursor 容忍空 hooks）。 */
        hooks.retain(|_, value| !value.as_array().is_some_and(|a| a.is_empty()));
        return Ok(());
    }
    for event in CURSOR_EVENTS {
        let entries = hooks
            .entry((*event).to_string())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{event} must be an array"))?;
        if entries.iter().any(|item| {
            item.get("command")
                .and_then(Value::as_str)
                .map(is_owned_command)
                .unwrap_or(false)
        }) {
            continue;
        }
        let mut hook = json!({
            "command": command,
            "timeout": 30,
        });
        if *event == "preToolUse" {
            hook["matcher"] = json!(CURSOR_PRE_TOOL_MATCHER);
        }
        entries.push(hook);
    }
    Ok(())
}

pub fn merge_hooks(config: &mut Value, command: &str, uninstall: bool) -> Result<(), String> {
    let root = config
        .as_object_mut()
        .ok_or_else(|| "hook config root must be a JSON object".to_string())?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| "hook config hooks must be an object".to_string())?;
    if uninstall {
        for value in hooks.values_mut() {
            remove_owned(value);
        }
        return Ok(());
    }
    for event in EVENTS {
        let entries = hooks
            .entry((*event).to_string())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{event} must be an array"))?;
        let mut added = false;
        for entry in entries.iter_mut() {
            if let Some(nested) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
                if !nested.iter().any(|item| {
                    item.get("command")
                        .and_then(Value::as_str)
                        .map(is_owned_command)
                        .unwrap_or(false)
                }) {
                    nested.push(json!({"type":"command", "command": command}));
                }
                added = true;
                break;
            }
        }
        if !added
            && !entries.iter().any(|item| {
                item.get("command")
                    .and_then(Value::as_str)
                    .map(is_owned_command)
                    .unwrap_or(false)
            })
        {
            entries.push(json!({
                "matcher": "*",
                "hooks": [{"type":"command", "command": command}]
            }));
        }
    }
    Ok(())
}

fn config_path(agent: &str) -> PathBuf {
    let env_key = match agent {
        "claude" => "TRELLIS_CARD_CLAUDE_CONFIG",
        "codex" => "TRELLIS_CARD_CODEX_CONFIG",
        "cursor" => "TRELLIS_CARD_CURSOR_CONFIG",
        _ => "TRELLIS_CARD_HOOK_CONFIG",
    };
    if let Ok(path) = std::env::var(env_key) {
        return PathBuf::from(path);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    default_config_path(agent, &home)
}

fn codex_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("TRELLIS_CARD_CODEX_CONFIG_TOML") {
        return PathBuf::from(path);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".codex/config.toml")
}

fn default_config_path(agent: &str, home: &Path) -> PathBuf {
    match agent {
        "claude" => home.join(".claude/settings.json"),
        "codex" => home.join(".codex/hooks.json"),
        "cursor" => home.join(".cursor/hooks.json"),
        _ => home.join(".config/trellis-card/hooks.json"),
    }
}

fn hook_command(executable: &Path, agent: &str) -> String {
    let path = executable.to_string_lossy();
    if cfg!(windows) {
        /* Hook 可能由 cmd.exe 或 PowerShell 执行。显式启动 PowerShell，避免
        `&` 在 cmd 中被当作命令分隔符；单引号路径可正确处理空格。 */
        let escaped_path = path.replace('\'', "''");
        format!(
            r#"powershell.exe -NoProfile -NonInteractive -Command "& '{}' hook --agent {}""#,
            escaped_path, agent
        )
    } else {
        format!(r#""{}" hook --agent {}"#, path, agent)
    }
}

pub fn install_hooks(agent: &str, uninstall: bool) -> Result<PathBuf, String> {
    if !matches!(agent, "claude" | "codex" | "cursor") {
        return Err("agent must be codex, claude or cursor".into());
    }
    let path = config_path(agent);
    let (mut config, config_exists) = load_json(&path)?;
    let mut codex_toml = None;
    if agent == "codex" && !uninstall {
        let toml_path = codex_config_path();
        let (mut toml, _) = load_toml(&toml_path)?;
        merge_codex_features(&mut toml)?;
        codex_toml = Some((toml_path, toml));
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let command = hook_command(&executable, agent);
    if agent == "cursor" {
        merge_cursor_hooks(&mut config, &command, uninstall)?;
    } else {
        merge_hooks(&mut config, &command, uninstall)?;
    }
    if !uninstall || config_exists {
        let data = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
        write_atomic(&path, &data)?;
    }
    if let Some((toml_path, toml)) = codex_toml {
        write_atomic(&toml_path, toml.to_string().as_bytes())?;
    }
    Ok(path)
}

pub fn status(agent: &str) -> Result<HookStatus, String> {
    if !matches!(agent, "codex" | "claude" | "cursor") {
        return Err("agent must be codex, claude or cursor".into());
    }
    let path = config_path(agent);
    let (config, config_exists) = load_json(&path)?;
    Ok(HookStatus {
        agent: agent.to_owned(),
        installed: config_exists && contains_owned(&config),
        config_exists,
        config_path: path.to_string_lossy().into_owned(),
    })
}

pub fn statuses() -> Result<Vec<HookStatus>, String> {
    ["codex", "claude", "cursor"]
        .into_iter()
        .map(status)
        .collect()
}

pub fn run_hook_install_cli() {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let agent = args
        .windows(2)
        .find(|pair| pair[0] == "--agent")
        .map(|pair| pair[1].as_str())
        .unwrap_or("codex");
    let uninstall = args.iter().any(|arg| arg == "--uninstall");
    match install_hooks(agent, uninstall) {
        Ok(path) => println!(
            "{} hooks {}: {}",
            agent,
            if uninstall { "removed" } else { "installed" },
            path.display()
        ),
        Err(error) => eprintln!("[hook] install failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn install_is_idempotent_and_preserves_foreign_hooks() {
        let mut config = json!({
            "model": "keep-me",
            "hooks": {
                "PreToolUse": [{"matcher":"*", "hooks":[{"type":"command","command":"echo foreign"}]}]
            }
        });
        let command = "\"/bin/trellis-card\" hook --agent codex";
        merge_hooks(&mut config, command, false).unwrap();
        merge_hooks(&mut config, command, false).unwrap();
        let hooks = config["hooks"]["PreToolUse"].as_array().unwrap();
        let commands = hooks[0]["hooks"].as_array().unwrap();
        assert_eq!(commands.len(), 2);
        merge_hooks(&mut config, command, false).unwrap();
        assert_eq!(config["model"], "keep-me");
        let commands = config["hooks"]["PreToolUse"][0]["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(commands.len(), 2);
    }

    #[test]
    fn uninstall_removes_only_owned_commands() {
        let mut config = json!({
            "hooks": {
                "Stop": [{"matcher":"*", "hooks":[
                    {"type":"command","command":"echo foreign"},
                    {"type":"command","command":"\"/bin/trellis-card\" hook --agent claude"}
                ]}]
            }
        });
        merge_hooks(&mut config, "/bin/trellis-card hook --agent claude", true).unwrap();
        let commands = config["hooks"]["Stop"][0]["hooks"].as_array().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["command"], "echo foreign");
    }

    #[test]
    fn ownership_matching_handles_quoted_executables() {
        assert!(is_owned_command("\"/opt/trellis-card\" hook --agent codex"));
        assert!(is_owned_command("/opt/trellis-card hook --agent codex"));
        assert!(!is_owned_command(
            "/opt/trellis-card-other hook --agent codex"
        ));
    }

    #[test]
    fn hook_command_uses_platform_shell_syntax() {
        let command = hook_command(
            Path::new(r#"C:\Program Files\Trellis-Card\trellis-card.exe"#),
            "claude",
        );
        if cfg!(windows) {
            assert_eq!(
                command,
                r#"powershell.exe -NoProfile -NonInteractive -Command "& 'C:\Program Files\Trellis-Card\trellis-card.exe' hook --agent claude""#
            );
        } else {
            assert_eq!(
                command,
                r#""C:\Program Files\Trellis-Card\trellis-card.exe" hook --agent claude"#
            );
        }
    }

    #[test]
    fn ownership_matching_handles_windows_exe_paths() {
        /* Windows: bin 名或 productName 重命名，带 .exe 扩展名、反斜杠路径 */
        assert!(is_owned_command(
            r#""C:\Users\foo\AppData\Local\Trellis-Card\trellis-card.exe" hook --agent codex"#
        ));
        assert!(is_owned_command(
            r#""C:\Users\foo\AppData\Local\Trellis-Card\Trellis-Card.exe" hook --agent claude"#
        ));
        assert!(is_owned_command(
            r"C:\Program Files\Trellis-Card\trellis-card.exe hook --agent codex"
        ));
        assert!(!is_owned_command(
            r#""C:\Users\foo\trellis-card-other.exe" hook --agent codex"#
        ));
        assert!(!is_owned_command("C:\\x\\trellis-card.exe install"));
    }

    #[test]
    fn contains_owned_finds_nested_hook_without_matching_foreign_commands() {
        let config = json!({
            "hooks": {
                "Stop": [{"hooks": [
                    {"type":"command","command":"echo foreign"},
                    {"type":"command","command":"/bin/trellis-card hook --agent codex"}
                ]}]
            }
        });
        assert!(contains_owned(&config));
        assert!(!contains_owned(&json!({"command": "echo foreign"})));
    }

    #[test]
    fn codex_uses_hooks_json_path() {
        let home = Path::new("/tmp/test-home");
        assert_eq!(
            default_config_path("codex", home),
            home.join(".codex/hooks.json")
        );
        assert_eq!(
            default_config_path("claude", home),
            home.join(".claude/settings.json")
        );
    }

    #[test]
    fn cursor_uses_hooks_json_path() {
        let home = Path::new("/tmp/test-home");
        assert_eq!(
            default_config_path("cursor", home),
            home.join(".cursor/hooks.json")
        );
    }

    #[test]
    fn cursor_merge_appends_owned_commands_and_preserves_foreign() {
        /* 结构：Trellis 生成的 Cursor hooks.json（扁平）+ 用户自定义 hook。 */
        let mut config = json!({
            "version": 1,
            "hooks": {
                "preToolUse": [{"command":"python3 .cursor/hooks/inject-subagent-context.py","matcher":"Task|Subagent","timeout":30}],
                "sessionStart": [{"command":"python3 .cursor/hooks/session-start.py","timeout":30}],
                "beforeShellExecution": [{"command":"python3 .cursor/hooks/inject-shell-session-context.py","timeout":5}]
            }
        });
        let command = "\"/bin/trellis-card\" hook --agent cursor";
        merge_cursor_hooks(&mut config, command, false).unwrap();
        assert_eq!(config["version"].as_u64(), Some(1));

        /* preToolUse：追加自有 command（Trellis 脚本保留） */
        let pt = config["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(pt.len(), 2);
        assert!(pt
            .iter()
            .any(|h| h["command"].as_str().map(is_owned_command).unwrap_or(false)));
        /* 新加的 preToolUse hook 带 matcher */
        let owned = pt
            .iter()
            .find(|h| h["command"].as_str().map(is_owned_command).unwrap_or(false))
            .unwrap();
        assert!(owned["matcher"].is_string());

        /* sessionStart / beforeShellExecution：追加自有 command */
        assert_eq!(config["hooks"]["sessionStart"].as_array().unwrap().len(), 2);
        assert_eq!(
            config["hooks"]["beforeShellExecution"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        /* stop / sessionEnd 原不存在 → 新建数组 */
        assert_eq!(config["hooks"]["stop"].as_array().unwrap().len(), 1);
        assert_eq!(config["hooks"]["sessionEnd"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn cursor_merge_is_idempotent() {
        let mut config = json!({ "version": 1, "hooks": {} });
        let command = "\"/bin/trellis-card\" hook --agent cursor";
        merge_cursor_hooks(&mut config, command, false).unwrap();
        merge_cursor_hooks(&mut config, command, false).unwrap();
        assert_eq!(config["hooks"]["sessionStart"].as_array().unwrap().len(), 1);
        assert_eq!(config["hooks"]["preToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(config["hooks"]["stop"].as_array().unwrap().len(), 1);
        assert_eq!(config["hooks"]["sessionEnd"].as_array().unwrap().len(), 1);
        assert_eq!(
            config["hooks"]["beforeShellExecution"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn cursor_uninstall_removes_only_owned_and_keeps_version() {
        let mut config = json!({
            "version": 1,
            "hooks": {
                "sessionStart": [
                    {"command":"python3 .cursor/hooks/session-start.py","timeout":30},
                    {"command":"\"/bin/trellis-card\" hook --agent cursor","timeout":30}
                ],
                "beforeShellExecution": [
                    {"command":"python3 .cursor/hooks/inject-shell-session-context.py","timeout":5}
                ]
            }
        });
        let command = "\"/bin/trellis-card\" hook --agent cursor";
        merge_cursor_hooks(&mut config, command, true).unwrap();
        /* 保留 version */
        assert_eq!(config["version"].as_u64(), Some(1));
        /* sessionStart：只删自有，Trellis 脚本保留 */
        let ss = config["hooks"]["sessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 1);
        assert_eq!(ss[0]["command"], "python3 .cursor/hooks/session-start.py");
        /* beforeShellExecution 无自有 command → 保留 Trellis 脚本，不误删 */
        let bse = config["hooks"]["beforeShellExecution"].as_array().unwrap();
        assert_eq!(bse.len(), 1);
        assert_eq!(
            bse[0]["command"],
            "python3 .cursor/hooks/inject-shell-session-context.py"
        );
    }

    #[test]
    fn cursor_status_detects_owned_command() {
        let config = json!({
            "version": 1,
            "hooks": {
                "stop": [{"command":"python3 .cursor/hooks/session-start.py","timeout":30}],
                "sessionEnd": [{"command":"\"/opt/trellis-card\" hook --agent cursor","timeout":30}]
            }
        });
        assert!(contains_owned(&config));
        assert!(!contains_owned(&json!({
            "version": 1,
            "hooks": {"stop": [{"command":"echo foreign"}]}
        })));
    }

    #[test]
    fn cursor_rejects_unsupported_version() {
        let mut config = json!({ "version": 2, "hooks": {} });
        let err = merge_cursor_hooks(&mut config, "/bin/trellis-card hook --agent cursor", false)
            .unwrap_err();
        assert!(err.contains("version"));
    }

    #[test]
    fn codex_features_enable_hooks_without_clobbering_config() {
        let mut config = "model = \"keep-me\"\n\n[features]\nexperimental = true\n"
            .parse::<Document>()
            .unwrap();
        merge_codex_features(&mut config).unwrap();
        assert_eq!(config["model"].as_str(), Some("keep-me"));
        assert_eq!(config["features"]["experimental"].as_bool(), Some(true));
        assert_eq!(config["features"]["hooks"].as_bool(), Some(true));
    }

    #[test]
    fn codex_features_support_inline_table() {
        let mut config = "[features]\n\nmodel = \"keep-me\"\n";
        let mut document = config.parse::<Document>().unwrap();
        merge_codex_features(&mut document).unwrap();
        assert_eq!(document["features"]["hooks"].as_bool(), Some(true));

        config = "features = { experimental = true }\n";
        document = config.parse::<Document>().unwrap();
        merge_codex_features(&mut document).unwrap();
        assert_eq!(document["features"]["experimental"].as_bool(), Some(true));
        assert_eq!(document["features"]["hooks"].as_bool(), Some(true));
    }
}
