use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

/* Pi 不是「hook + JSON 配置」平台，而是「extension + TS 事件」平台。
桥接扩展以用户级全局目录 ~/.pi/agent/extensions/ 安装（所有项目生效），
Trellis Card 写入自有扩展文件，卸载时只删该文件，保留同目录其他扩展。 */
const PI_EXTENSIONS_DIR: &str = ".pi/agent/extensions";
const PI_BRIDGE_FILE: &str = "trellis-card.ts";
const PI_BRIDGE_TEMPLATE: &str = include_str!("../templates/pi_bridge.ts");

/* OpenCode 同样是「plugin + JS 事件」平台（非 JSON hook 配置）。
观察者 plugin 以用户级全局目录 ~/.config/opencode/plugins/ 安装（所有项目生效，
自动发现，无需项目信任）。与 Pi 对称：写入自有 plugin 文件，卸载只删该文件。 */
const OPENCODE_PLUGINS_DIR: &str = ".config/opencode/plugins";
const OPENCODE_PLUGIN_FILE: &str = "trellis-card.js";
const OPENCODE_PLUGIN_TEMPLATE: &str = include_str!("../templates/opencode_plugin.js");

/* DSH（DeepSeek Harness）不是「hook + JSON 配置」平台，而是 cordis 插件体系。
Trellis Card 内置一个 dsh-trellis-bridge cordis 插件（多文件 npm 包），安装时
复制到 ~/.config/trellis-card/agents/dsh-trellis-bridge/ 并用 `dsh plugin
--profile web add link:<dir>` 挂载到 dsh web profile；卸载时 `dsh plugin
--profile web remove` 并从 profile 移除。模板内嵌在 src-tauri/templates/dsh/。 */
const DSH_PROFILE: &str = "web";
const DSH_BRIDGE_PACKAGE: &str = "dsh-trellis-bridge";
/* 目录名必须等于 npm 包名 dsh-trellis-bridge：`dsh plugin add link:<dir>` 用目录名
当包名注册，目录名不符会导致 profile 里出现错误的包名。 */
const DSH_AGENTS_DIR: &str = "agents/dsh-trellis-bridge";
const DSH_INDEX_TEMPLATE: &str = include_str!("../templates/dsh/src/index.js");
const DSH_LIB_TEMPLATE: &str = include_str!("../templates/dsh/src/lib.js");
const DSH_PACKAGE_JSON_TEMPLATE: &str = include_str!("../templates/dsh/package.json");
const DSH_PATCH_TEMPLATE: &str = include_str!("../templates/dsh/cordis.patch.yml");

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

/* 独立文件型 agent（Pi 扩展 / OpenCode plugin）：安装=写入自有文件（幂等，保留
同目录其他文件），卸载=删除自有文件。原子写入避免运行时读到半截文件。 */
fn write_standalone_file(path: &Path, template: &str, uninstall: bool) -> Result<(), String> {
    if uninstall {
        if path.exists() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
    } else if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        write_atomic(path, template.as_bytes())?;
    } else {
        write_atomic(path, template.as_bytes())?;
    }
    Ok(())
}

/* ---- DSH bridge（cordis 插件包）---- */

/** Where the dsh-trellis-bridge plugin package lives on disk. */
fn dsh_bridge_dir() -> PathBuf {
    if let Ok(path) = std::env::var("TRELLIS_CARD_DSH_DIR") {
        return PathBuf::from(path);
    }
    crate::config::app_config_dir().join(DSH_AGENTS_DIR)
}

/** Write the embedded dsh-trellis-bridge package to the target directory. */
fn write_dsh_bridge(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    fs::write(dir.join("package.json"), DSH_PACKAGE_JSON_TEMPLATE).map_err(|e| e.to_string())?;
    fs::write(dir.join("cordis.patch.yml"), DSH_PATCH_TEMPLATE).map_err(|e| e.to_string())?;
    fs::write(dir.join("src/index.js"), DSH_INDEX_TEMPLATE).map_err(|e| e.to_string())?;
    fs::write(dir.join("src/lib.js"), DSH_LIB_TEMPLATE).map_err(|e| e.to_string())?;
    Ok(())
}

/** Locate the dsh executable across common install roots and $PATH.
GUI 应用由 LaunchServices 启动，PATH 通常不含 /opt/homebrew/bin（Homebrew），
而 dsh 装在那里——用绝对路径探测 + PATH 兜底，避免 Command::new("dsh") 找不到。 */
fn dsh_bin() -> Option<PathBuf> {
    let candidates = [
        "/opt/homebrew/bin/dsh",
        "/usr/local/bin/dsh",
        "/usr/bin/dsh",
        "$HOME/.local/bin/dsh",
    ];
    for raw in candidates {
        let path = PathBuf::from(raw.replace(
            "$HOME",
            &dirs::home_dir().unwrap_or_default().to_string_lossy(),
        ));
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let candidate = PathBuf::from(dir).join("dsh");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/** Run `dsh plugin --profile <p> <args...>` and return stdout/stderr on failure. */
fn dsh_plugin(args: &[&str]) -> Result<String, String> {
    let bin = dsh_bin().ok_or_else(|| {
        "无法定位 dsh CLI：请确认已安装 DeepSeek Harness（npm i -g @deepseek-ai/dsh）".to_string()
    })?;
    let output = Command::new(&bin)
        .args(["plugin", "--profile", DSH_PROFILE])
        .args(args)
        .output()
        .map_err(|e| format!("无法执行 dsh CLI：{e}（请确认已安装 DeepSeek Harness）"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).into_owned();
        let out = String::from_utf8_lossy(&output.stdout).into_owned();
        Err(format!("dsh plugin 失败：{err}{out}"))
    }
}

/** Install the bridge package and mount it into the dsh web profile. */
fn install_dsh_bridge() -> Result<PathBuf, String> {
    let dir = dsh_bridge_dir();
    write_dsh_bridge(&dir)?;
    let link = format!("link:{}", dir.display());
    dsh_plugin(&["add", &link])?;
    Ok(dir)
}

/** Unmount the bridge from the dsh profile and remove the package directory. */
fn uninstall_dsh_bridge() -> Result<(), String> {
    let _ = dsh_plugin(&["remove", DSH_BRIDGE_PACKAGE]);
    let dir = dsh_bridge_dir();
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/** The dsh profile manifest that lists bundles (test-overridable). */
fn dsh_profile_manifest() -> PathBuf {
    if let Ok(path) = std::env::var("TRELLIS_CARD_DSH_PROFILE_MANIFEST") {
        return PathBuf::from(path);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".dsh")
        .join("profiles")
        .join(DSH_PROFILE)
        .join("package.json")
}

/** True when the bridge package dir exists AND is listed in the profile bundles. */
fn dsh_installed() -> bool {
    if !dsh_bridge_dir().join("package.json").is_file() {
        return false;
    }
    /* 解析 manifest 的 dsh.profile.bundles 数组，避免子串误判（用户在其他字段写同名）。 */
    let text = match std::fs::read_to_string(dsh_profile_manifest()) {
        Ok(text) => text,
        Err(_) => return false,
    };
    let value: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => return false,
    };
    value
        .get("dsh")
        .and_then(|dsh| dsh.get("profile"))
        .and_then(|profile| profile.get("bundles"))
        .and_then(Value::as_array)
        .map(|bundles| {
            bundles
                .iter()
                .any(|b| b.as_str() == Some(DSH_BRIDGE_PACKAGE))
        })
        .unwrap_or(false)
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
    if agent == "dsh" {
        return dsh_bridge_dir();
    }
    let env_key = match agent {
        "claude" => "TRELLIS_CARD_CLAUDE_CONFIG",
        "codex" => "TRELLIS_CARD_CODEX_CONFIG",
        "cursor" => "TRELLIS_CARD_CURSOR_CONFIG",
        "pi" => "TRELLIS_CARD_PI_EXTENSIONS_FILE",
        "opencode" => "TRELLIS_CARD_OPENCODE_PLUGIN_FILE",
        _ => "TRELLIS_CARD_HOOK_CONFIG",
    };
    if let Ok(path) = std::env::var(env_key) {
        return PathBuf::from(path);
    }
    /* WSL 观察模式：配置写到 WSL 侧 agent 的 home（\\wsl$\<distro>\home\<user>\...）。
    子进程 hook 或应用侧均可设置 TRELLIS_CARD_WSL_DISTRO 触发。 */
    if let Some(distro) = crate::platform::wsl_distro() {
        if let Some(unc_home) = wsl_home_unc(&distro) {
            return default_config_path(agent, &unc_home);
        }
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    default_config_path(agent, &home)
}

fn codex_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("TRELLIS_CARD_CODEX_CONFIG_TOML") {
        return PathBuf::from(path);
    }
    if let Some(distro) = crate::platform::wsl_distro() {
        if let Some(unc_home) = wsl_home_unc(&distro) {
            return unc_home.join(".codex/config.toml");
        }
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".codex/config.toml")
}

/* WSL 观察模式下的用户 home UNC：`\\wsl$\<distro>\home\<user>`。
/root 用户映射到 `\\wsl$\<distro>\root`（WSL 默认 root 家目录在 /root）。
env TRELLIS_CARD_WSL_HOME 优先（测试注入 + 用户显式覆盖，如 root 或非标准 home）；
未设置时 Windows 上运行 wsl.exe 读 WSL 内 HOME，其他平台返回 None。 */
fn wsl_home_unc(distro: &str) -> Option<PathBuf> {
    let _ = distro; // 非 Windows 分支不使用；Windows 分支 shadow 后使用
    if let Ok(home) = std::env::var("TRELLIS_CARD_WSL_HOME") {
        let home = home.trim().to_string();
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    #[cfg(windows)]
    {
        let distro = distro.trim().to_string();
        if distro.is_empty() {
            return None;
        }
        /* 运行 wsl.exe 取 WSL 内 HOME（如 /home/alice）；失败时回退 /root。
        用 `wsl.exe -d <distro> -e sh -c 'echo $HOME'` 读取。 */
        let output = std::process::Command::new("wsl.exe")
            .args(["-d", &distro, "-e", "sh", "-c", "echo $HOME"])
            .output();
        let home = output
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                crate::platform::decode_wsl_output(&o.stdout)
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/root".to_string());
        crate::platform::wsl_unc_from_linux(&home, &distro).map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn default_config_path(agent: &str, home: &Path) -> PathBuf {
    match agent {
        "claude" => home.join(".claude/settings.json"),
        "codex" => home.join(".codex/hooks.json"),
        "cursor" => home.join(".cursor/hooks.json"),
        "pi" => home.join(PI_EXTENSIONS_DIR).join(PI_BRIDGE_FILE),
        "opencode" => home.join(OPENCODE_PLUGINS_DIR).join(OPENCODE_PLUGIN_FILE),
        _ => home.join(".config/trellis-card/hooks.json"),
    }
}

fn hook_command(executable: &Path, agent: &str) -> String {
    /* WSL 观察模式：hook 由 WSL 内 agent 触发，执行的是 Windows 侧 trellis-card.exe。
    agent 在 WSL（bash）里 spawn 命令，exe 路径必须是 WSL 挂载形式 /mnt/c/...，
    用 bash 语法（引号路径 + 反斜杠不转义）。 */
    if crate::platform::wsl_distro().is_some() {
        let path = executable.to_string_lossy();
        if let Some(wsl_path) = crate::platform::windows_to_wsl_path(&path) {
            return format!(r#""{wsl_path}" hook --agent {agent}"#);
        }
    }
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
    if !matches!(
        agent,
        "claude" | "codex" | "cursor" | "pi" | "opencode" | "dsh"
    ) {
        return Err("agent must be codex, claude, cursor, pi, opencode or dsh".into());
    }
    /* DSH：cordis 插件包，非 JSON hook。安装=复制内置 bridge + dsh plugin add，
    卸载=dsh plugin remove + 删目录。 */
    if agent == "dsh" {
        if uninstall {
            uninstall_dsh_bridge()?;
        } else {
            install_dsh_bridge()?;
        }
        return Ok(dsh_bridge_dir());
    }
    /* Pi / OpenCode：无 JSON hook 配置，桥接文件是用户级目录里的独立文件。
    安装=写入自有文件（幂等），卸载=删除自有文件（保留同目录其他文件）。 */
    if agent == "pi" || agent == "opencode" {
        let path = config_path(agent);
        let template = if agent == "pi" {
            PI_BRIDGE_TEMPLATE
        } else {
            OPENCODE_PLUGIN_TEMPLATE
        };
        write_standalone_file(&path, template, uninstall)?;
        return Ok(path);
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
    if !matches!(
        agent,
        "codex" | "claude" | "cursor" | "pi" | "opencode" | "dsh"
    ) {
        return Err("agent must be codex, claude, cursor, pi, opencode or dsh".into());
    }
    let path = config_path(agent);
    if agent == "dsh" {
        let installed = dsh_installed();
        return Ok(HookStatus {
            agent: agent.to_owned(),
            installed,
            config_exists: dsh_bridge_dir().join("package.json").is_file(),
            config_path: dsh_bridge_dir().to_string_lossy().into_owned(),
        });
    }
    if agent == "pi" || agent == "opencode" {
        /* Pi / OpenCode 的「配置」就是桥接文件本身：installed = 文件存在。 */
        let installed = path.is_file();
        return Ok(HookStatus {
            agent: agent.to_owned(),
            installed,
            config_exists: installed,
            config_path: path.to_string_lossy().into_owned(),
        });
    }
    let (config, config_exists) = load_json(&path)?;
    Ok(HookStatus {
        agent: agent.to_owned(),
        installed: config_exists && contains_owned(&config),
        config_exists,
        config_path: path.to_string_lossy().into_owned(),
    })
}

pub fn statuses() -> Result<Vec<HookStatus>, String> {
    ["codex", "claude", "cursor", "pi", "opencode", "dsh"]
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
    use std::sync::Mutex;

    /* DSH 测试共享 TRELLIS_CARD_DSH_DIR env，必须串行，避免并行时互相覆盖。 */
    static DSH_TEST_LOCK: Mutex<()> = Mutex::new(());

    /* WSL 测试设置/清除 TRELLIS_CARD_WSL_DISTRO 等 env，会与其他依赖这些 env
    的测试（含原有 hook_command 平台语法测试）竞态，必须串行。 */
    static WSL_TEST_LOCK: Mutex<()> = Mutex::new(());

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
        /* WSL env 会改变 hook_command 输出，加锁避免并行竞态。 */
        let _guard = WSL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("TRELLIS_CARD_WSL_DISTRO");
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

    /* ---- Pi 桥接扩展（用户级 ~/.pi/agent/extensions/trellis-card.ts） ---- */

    #[test]
    fn pi_uses_user_level_extensions_file_path() {
        let home = Path::new("/tmp/test-home");
        assert_eq!(
            default_config_path("pi", home),
            home.join(".pi/agent/extensions/trellis-card.ts")
        );
    }

    #[test]
    fn pi_install_writes_bridge_file_and_status_detects_it() {
        let home =
            std::env::temp_dir().join(format!("trellis-card-pi-install-{}", std::process::id()));
        std::env::set_var(
            "TRELLIS_CARD_PI_EXTENSIONS_FILE",
            home.join("trellis-card.ts"),
        );
        let path = config_path("pi");
        let _ = std::fs::remove_file(&path);

        assert!(!path.exists());
        let before = status("pi").unwrap();
        assert!(!before.installed);

        install_hooks("pi", false).unwrap();
        let after = status("pi").unwrap();
        assert!(after.installed);
        assert_eq!(after.config_path, path.to_string_lossy());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Trellis Card bridge extension for Pi"));
        assert!(content.contains("export default function (pi: any)"));

        /* 幂等：重复安装不报错、不重复 */
        install_hooks("pi", false).unwrap();
        let same = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, same);

        /* 卸载：只删自有文件，同目录其他扩展保留 */
        std::fs::write(
            home.join("other-extension.ts"),
            "export default function(pi){}\n",
        )
        .unwrap();
        install_hooks("pi", true).unwrap();
        assert!(!path.exists());
        assert!(home.join("other-extension.ts").exists());
        let after_uninstall = status("pi").unwrap();
        assert!(!after_uninstall.installed);

        let _ = std::fs::remove_dir_all(&home);
        std::env::remove_var("TRELLIS_CARD_PI_EXTENSIONS_FILE");
    }

    #[test]
    fn pi_rejects_unknown_agent() {
        assert!(install_hooks("unknown", false).is_err());
        assert!(status("unknown").is_err());
    }

    #[test]
    fn opencode_uses_user_level_plugins_file_path() {
        let home = Path::new("/tmp/test-home");
        assert_eq!(
            default_config_path("opencode", home),
            home.join(".config/opencode/plugins/trellis-card.js")
        );
    }

    #[test]
    fn opencode_install_writes_plugin_and_status_detects_it() {
        let home = std::env::temp_dir().join(format!(
            "trellis-card-opencode-install-{}",
            std::process::id()
        ));
        std::env::set_var(
            "TRELLIS_CARD_OPENCODE_PLUGIN_FILE",
            home.join("trellis-card.js"),
        );
        let path = config_path("opencode");
        let _ = std::fs::remove_file(&path);

        assert!(!path.exists());
        let before = status("opencode").unwrap();
        assert!(!before.installed);

        install_hooks("opencode", false).unwrap();
        let after = status("opencode").unwrap();
        assert!(after.installed);
        assert_eq!(after.config_path, path.to_string_lossy());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Trellis Card bridge plugin for OpenCode"));
        assert!(content.contains("chat.message"));
        assert!(content.contains("tool.execute.before"));

        /* 幂等：重复安装不报错、不重复 */
        install_hooks("opencode", false).unwrap();
        let same = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, same);

        /* 卸载：只删自有文件，同目录其他 plugin 保留 */
        std::fs::write(home.join("other-plugin.js"), "export default async()=>{}\n").unwrap();
        install_hooks("opencode", true).unwrap();
        assert!(!path.exists());
        assert!(home.join("other-plugin.js").exists());
        let after_uninstall = status("opencode").unwrap();
        assert!(!after_uninstall.installed);

        let _ = std::fs::remove_dir_all(&home);
        std::env::remove_var("TRELLIS_CARD_OPENCODE_PLUGIN_FILE");
    }

    #[test]
    fn opencode_rejects_unknown_agent() {
        assert!(install_hooks("unknown", false).is_err());
        assert!(status("unknown").is_err());
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

    /* ---- DSH bridge（cordis 插件包）---- */

    #[test]
    fn dsh_config_path_default_uses_dsh_trellis_bridge_dir() {
        let _guard = DSH_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("TRELLIS_CARD_DSH_DIR");
        let path = config_path("dsh");
        /* 默认目录必须指向 agents/dsh-trellis-bridge（目录名 = npm 包名，dsh plugin link 依赖它） */
        assert!(
            path.to_string_lossy().ends_with("dsh-trellis-bridge"),
            "dsh default config path should end with dsh-trellis-bridge, got {}",
            path.display()
        );
        assert!(path.to_string_lossy().contains("agents"));
    }

    #[test]
    fn dsh_config_path_env_override_wins() {
        let _guard = DSH_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root =
            std::env::temp_dir().join(format!("trellis-card-dsh-cfg-{}", std::process::id()));
        std::env::set_var(
            "TRELLIS_CARD_DSH_DIR",
            root.join("override-dir").display().to_string(),
        );
        let path = config_path("dsh");
        assert_eq!(path, root.join("override-dir"));
        std::env::remove_var("TRELLIS_CARD_DSH_DIR");
    }

    #[test]
    fn dsh_write_bridge_writes_all_four_files() {
        let _guard = DSH_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root =
            std::env::temp_dir().join(format!("trellis-card-dsh-write-{}", std::process::id()));
        std::env::set_var("TRELLIS_CARD_DSH_DIR", root.display().to_string());
        let dir = dsh_bridge_dir();
        let _ = std::fs::remove_dir_all(&dir);
        write_dsh_bridge(&dir).unwrap();
        assert!(dir.join("package.json").is_file());
        assert!(dir.join("cordis.patch.yml").is_file());
        assert!(dir.join("src/index.js").is_file());
        assert!(dir.join("src/lib.js").is_file());
        let pkg = std::fs::read_to_string(dir.join("package.json")).unwrap();
        assert!(pkg.contains("dsh-trellis-bridge"));
        let index = std::fs::read_to_string(dir.join("src/index.js")).unwrap();
        assert!(index.contains("session/event"));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::remove_var("TRELLIS_CARD_DSH_DIR");
    }

    #[test]
    fn dsh_status_reports_not_installed_without_manifest() {
        let _guard = DSH_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("trellis-card-dsh-st-{}", std::process::id()));
        std::env::set_var(
            "TRELLIS_CARD_DSH_DIR",
            root.join("bridge").display().to_string(),
        );
        let manifest = root.join("profile-package.json");
        std::env::set_var(
            "TRELLIS_CARD_DSH_PROFILE_MANIFEST",
            manifest.display().to_string(),
        );
        let dir = dsh_bridge_dir();
        let _ = std::fs::remove_dir_all(&dir);
        write_dsh_bridge(&dir).unwrap();
        /* 无 profile manifest -> 未安装 */
        let st = status("dsh").unwrap();
        assert_eq!(st.agent, "dsh");
        assert!(!st.installed);
        assert!(st.config_exists);
        let _ = std::fs::remove_dir_all(&dir);
        std::env::remove_var("TRELLIS_CARD_DSH_DIR");
        std::env::remove_var("TRELLIS_CARD_DSH_PROFILE_MANIFEST");
    }

    #[test]
    fn dsh_installed_true_when_profile_manifest_lists_bridge() {
        let _guard = DSH_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("trellis-card-dsh-in-{}", std::process::id()));
        std::env::set_var(
            "TRELLIS_CARD_DSH_DIR",
            root.join("bridge").display().to_string(),
        );
        let manifest = root.join("profile-package.json");
        std::env::set_var(
            "TRELLIS_CARD_DSH_PROFILE_MANIFEST",
            manifest.display().to_string(),
        );
        let dir = dsh_bridge_dir();
        let _ = std::fs::remove_dir_all(&dir);
        write_dsh_bridge(&dir).unwrap();
        /* 构造 profile manifest，bundles 含 dsh-trellis-bridge */
        std::fs::write(
            &manifest,
            r#"{"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","dsh-trellis-bridge"]}}}"#,
        )
        .unwrap();
        assert!(dsh_installed());
        let st = status("dsh").unwrap();
        assert!(st.installed);
        /* bundles 不含 bridge -> 未安装 */
        std::fs::write(
            &manifest,
            r#"{"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base"]}}}"#,
        )
        .unwrap();
        assert!(!dsh_installed());
        let _ = std::fs::remove_dir_all(&root);
        std::env::remove_var("TRELLIS_CARD_DSH_DIR");
        std::env::remove_var("TRELLIS_CARD_DSH_PROFILE_MANIFEST");
    }

    #[test]
    fn dsh_status_rejects_unknown_agent() {
        assert!(install_hooks("unknown", false).is_err());
        assert!(status("unknown").is_err());
    }

    /* ---- WSL 观察模式：配置路径与 hook command ---- */

    #[test]
    fn config_path_uses_wsl_home_unc_when_distro_configured() {
        let _guard = WSL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("TRELLIS_CARD_WSL_DISTRO", "Ubuntu");
        std::env::set_var("TRELLIS_CARD_WSL_HOME", r"\\wsl$\Ubuntu\home\alice");
        let posix = |p: &PathBuf| crate::platform::to_posix(&p.to_string_lossy());
        assert_eq!(
            posix(&config_path("claude")),
            "//wsl$/Ubuntu/home/alice/.claude/settings.json"
        );
        assert_eq!(
            posix(&config_path("codex")),
            "//wsl$/Ubuntu/home/alice/.codex/hooks.json"
        );
        assert_eq!(
            posix(&config_path("cursor")),
            "//wsl$/Ubuntu/home/alice/.cursor/hooks.json"
        );
        assert_eq!(
            posix(&config_path("pi")),
            "//wsl$/Ubuntu/home/alice/.pi/agent/extensions/trellis-card.ts"
        );
        assert_eq!(
            posix(&config_path("opencode")),
            "//wsl$/Ubuntu/home/alice/.config/opencode/plugins/trellis-card.js"
        );
        /* env 覆盖仍优先于 WSL 推导 */
        std::env::set_var("TRELLIS_CARD_CLAUDE_CONFIG", r"C:\custom\settings.json");
        assert_eq!(
            config_path("claude").to_string_lossy(),
            r"C:\custom\settings.json"
        );
        std::env::remove_var("TRELLIS_CARD_WSL_DISTRO");
        std::env::remove_var("TRELLIS_CARD_WSL_HOME");
        std::env::remove_var("TRELLIS_CARD_CLAUDE_CONFIG");
    }

    #[test]
    fn codex_toml_path_uses_wsl_home_when_distro_configured() {
        let _guard = WSL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("TRELLIS_CARD_WSL_DISTRO", "Ubuntu");
        std::env::set_var("TRELLIS_CARD_WSL_HOME", r"\\wsl$\Ubuntu\home\alice");
        assert_eq!(
            crate::platform::to_posix(&codex_config_path().to_string_lossy()),
            "//wsl$/Ubuntu/home/alice/.codex/config.toml"
        );
        std::env::set_var("TRELLIS_CARD_CODEX_CONFIG_TOML", r"C:\codex\config.toml");
        assert_eq!(
            codex_config_path().to_string_lossy(),
            r"C:\codex\config.toml"
        );
        std::env::remove_var("TRELLIS_CARD_WSL_DISTRO");
        std::env::remove_var("TRELLIS_CARD_WSL_HOME");
        std::env::remove_var("TRELLIS_CARD_CODEX_CONFIG_TOML");
    }

    #[test]
    fn hook_command_uses_wsl_mount_path_when_distro_configured() {
        let _guard = WSL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("TRELLIS_CARD_WSL_DISTRO", "Ubuntu");
        let exe = Path::new(r"C:\Program Files\Trellis-Card\trellis-card.exe");
        let command = hook_command(exe, "claude");
        assert_eq!(
            command,
            r#""/mnt/c/Program Files/Trellis-Card/trellis-card.exe" hook --agent claude"#
        );
        std::env::remove_var("TRELLIS_CARD_WSL_DISTRO");
    }

    #[test]
    fn hook_command_without_wsl_uses_platform_shell() {
        /* WSL env 会改变 hook_command 输出，加锁避免并行竞态。 */
        let _guard = WSL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("TRELLIS_CARD_WSL_DISTRO");
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
    fn dsh_bin_finds_executable_when_installed() {
        /* 本机若装了 dsh，应能定位到（PATH 或常见安装根）。未装则返回 None。 */
        let found = dsh_bin();
        if let Some(bin) = found {
            assert!(
                bin.is_file(),
                "dsh_bin should point at an existing file: {}",
                bin.display()
            );
        }
    }
}
