use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, Document};

const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Stop",
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
    command
        .replace(['"', '\''], "")
        .contains("trellis-card hook")
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
        _ => home.join(".config/trellis-card/hooks.json"),
    }
}

pub fn install_hooks(agent: &str, uninstall: bool) -> Result<PathBuf, String> {
    if !matches!(agent, "claude" | "codex") {
        return Err("agent must be codex or claude".into());
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
    let command = format!(
        "\"{}\" hook --agent {}",
        executable.to_string_lossy(),
        agent
    );
    merge_hooks(&mut config, &command, uninstall)?;
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
    if !matches!(agent, "codex" | "claude") {
        return Err("agent must be codex or claude".into());
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
    ["codex", "claude"].into_iter().map(status).collect()
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
