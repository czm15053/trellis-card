use crate::activity::{classify_tool, extract_task_dir};
use crate::config;
use crate::ipc;
use crate::runtime::ToolActivityInput;
use crate::session::HookEvent;
use serde_json::Value;
use std::io::Read;

#[derive(Debug, Clone, Default)]
pub struct HookOverrides {
    pub agent: Option<String>,
    pub event: Option<String>,
    pub session: Option<String>,
    pub project: Option<String>,
    pub task: Option<String>,
}

fn value_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(str::to_owned))
}

fn tool_input_value(value: &Value) -> Option<&Value> {
    value.get("tool_input").or_else(|| value.get("toolInput"))
}

fn tool_input_field(input: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str).map(str::to_owned))
}

fn tool_activity_input(value: &Value) -> Option<ToolActivityInput> {
    let input = tool_input_value(value)?;
    let tool_input = ToolActivityInput {
        file_path: tool_input_field(input, &["file_path", "filePath"]),
        command: tool_input_field(input, &["command", "cmd", "script"]),
        description: tool_input_field(input, &["description"]),
        pattern: tool_input_field(input, &["pattern"]),
        query: tool_input_field(input, &["query"]),
        url: tool_input_field(input, &["url"]),
        prompt: tool_input_field(input, &["prompt"]),
        subagent_type: tool_input_field(input, &["subagent_type", "subagentType"]),
    };
    (tool_input != ToolActivityInput::default()).then_some(tool_input)
}

fn command_from_payload(value: &Value) -> String {
    let input = tool_input_value(value);
    if let Some(input) = input {
        if let Some(command) = input.as_str() {
            return command.to_owned();
        }
        if let Some(command) = value_string(input, &["command", "cmd", "script"]) {
            return command;
        }
    }
    value_string(value, &["command", "cmd"]).unwrap_or_default()
}

/* Claude's non-shell tools carry their useful target in fields such as
file_path, path, pattern, or query. Keep this text small and deterministic so
the card can show activity and task binding does not depend on a Bash command. */
fn tool_input_text(value: &Value) -> String {
    let Some(input) = tool_input_value(value) else {
        return String::new();
    };
    if let Some(text) = input.as_str() {
        return text.to_owned();
    }
    if let Some(text) = value_string(
        input,
        &[
            "command",
            "cmd",
            "script",
            "file_path",
            "path",
            "pattern",
            "query",
            "url",
            "prompt",
            "description",
        ],
    ) {
        return text;
    }
    serde_json::to_string(input).unwrap_or_default()
}

fn event_activity(value: &Value, tool_name: Option<&str>) -> Option<String> {
    if let Some(text) = value_string(value, &["prompt", "message", "input"]) {
        return Some(text);
    }
    tool_name.map(str::to_owned)
}

fn normalize_task_id(value: String) -> String {
    extract_task_dir(&value).unwrap_or(value)
}

fn project_root_for_path(path: &str) -> String {
    let mut current = std::path::PathBuf::from(path);
    if !current.is_dir() {
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        }
    }
    loop {
        if current.join(".trellis").join("tasks").is_dir() {
            /* canonicalize + 去 Windows \\?\ 前缀，与 lib.rs 的项目路径保持一致 */
            return crate::platform::normalize_path(&current)
                .to_string_lossy()
                .into_owned();
        }
        if !current.pop() {
            return path.to_owned();
        }
    }
}

fn looks_like_task_id(value: &str) -> bool {
    let mut parts = value.splitn(2, '-');
    let prefix = parts.next().unwrap_or_default();
    let suffix = parts.next().unwrap_or_default();
    prefix.len() == 2 && prefix.chars().all(|c| c.is_ascii_digit()) && !suffix.is_empty()
}

fn extract_task_arg(command: &str, tool_name: Option<&str>) -> Option<String> {
    let tokens: Vec<String> = command
        .split_whitespace()
        .map(|token| {
            let token = token.trim_matches(|c: char| "'\"()[]{}<>,;".contains(c));
            crate::platform::to_posix(token)
        })
        .collect();
    let tool = tool_name.filter(|name| name.starts_with("trellis-"));
    for (index, token) in tokens.iter().enumerate() {
        let is_trellis_tool = tool.is_some_and(|name| token == name);
        let is_task_script = token == "task.py" || token.ends_with("/task.py");
        if !is_trellis_tool && !is_task_script {
            continue;
        }
        let start = index + usize::from(is_task_script) + 1;
        if let Some(candidate) = tokens.get(start) {
            if looks_like_task_id(candidate) {
                return Some(candidate.clone());
            }
        }
    }
    None
}

fn runtime_task_for_session(project: &str, session_id: &str) -> Option<String> {
    if project.is_empty() || session_id.is_empty() || session_id == "unknown-session" {
        return None;
    }
    let sessions_dir = std::path::Path::new(project)
        .join(".trellis")
        .join(".runtime")
        .join("sessions");
    let entries = std::fs::read_dir(sessions_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if name != session_id && !name.trim_end_matches(".json").ends_with(session_id) {
            continue;
        }
        let content = std::fs::read_to_string(path).ok()?;
        let value: Value = serde_json::from_str(&content).ok()?;
        return value
            .get("current_task")
            .or_else(|| value.get("currentTask"))
            .and_then(Value::as_str)
            .and_then(extract_task_dir);
    }
    None
}

/* Claude may rotate or omit the session id in a hook payload while Trellis
still has one active session pointer for the project. Use the newest valid
project pointer as a task binding fallback so activity reaches a task card. */
fn runtime_task_for_project(project: &str) -> Option<String> {
    if project.is_empty() {
        return None;
    }
    let sessions_dir = std::path::Path::new(project)
        .join(".trellis")
        .join(".runtime")
        .join("sessions");
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(sessions_dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        let Some(task) = value
            .get("current_task")
            .or_else(|| value.get("currentTask"))
            .and_then(Value::as_str)
            .and_then(extract_task_dir)
        else {
            continue;
        };
        if !std::path::Path::new(project)
            .join(".trellis")
            .join("tasks")
            .join(&task)
            .is_dir()
        {
            continue;
        }
        let last_seen = value
            .get("last_seen_at")
            .or_else(|| value.get("lastSeenAt"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        candidates.push((last_seen, task));
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.pop().map(|(_, task)| task)
}

/* CLI fallback：.trellis/.current-task 文本指针（session pointer 优先，这里只兜底）。
内容可为 .trellis/tasks/<id> 路径或裸 task id；统一规范化为任务目录，
且必须指向实际存在的 .trellis/tasks/<dir>。不伪造 SessionInfo/活跃时间，
只补充 task 解析，不改前端协议。 */
fn current_task_fallback(project: &str) -> Option<String> {
    if project.is_empty() {
        return None;
    }
    let pointer = std::path::Path::new(project)
        .join(".trellis")
        .join(".current-task");
    let text = std::fs::read_to_string(pointer).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let task = extract_task_dir(text).or_else(|| {
        /* 裸 task id：按 looks_like_task_id 格式校验，防止把任意文本当任务 */
        looks_like_task_id(text).then(|| text.to_owned())
    })?;
    let dir = std::path::Path::new(project)
        .join(".trellis")
        .join("tasks")
        .join(&task);
    dir.is_dir().then_some(task)
}

fn payload_path_belongs_to_project(project: &str, token: &str) -> bool {
    // Windows 路径用反斜杠分隔，先归一化再查找 `.trellis/tasks/` 子串
    let token = crate::platform::to_posix(token);
    let Some(marker) = token.find(".trellis/tasks/") else {
        return false;
    };
    let prefix = token[..marker].trim_end_matches('/');
    if prefix.is_empty() || prefix == "." {
        return true;
    }
    let candidate = std::path::Path::new(prefix);
    if !candidate.is_absolute() {
        return false;
    }
    let project = std::path::Path::new(project);
    let candidate = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());
    let project = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());
    crate::platform::path_eq_ignore_case(&candidate.to_string_lossy(), &project.to_string_lossy())
}

fn task_dir_from_payload(project: &str, input: &str) -> Option<String> {
    let project_path = std::path::Path::new(project);
    for raw in input.split_whitespace() {
        let token = raw.trim_matches(|c: char| "'\"()[]{}<>,;".contains(c));
        let Some(task) = extract_task_dir(token) else {
            continue;
        };
        if !payload_path_belongs_to_project(project, token) {
            continue;
        }
        let path = project_path.join(".trellis").join("tasks").join(&task);
        if path.is_dir() {
            return Some(task);
        }
    }
    None
}

pub fn parse_hook_payload(input: &str, overrides: &HookOverrides) -> Result<HookEvent, String> {
    let value: Value = serde_json::from_str(input).map_err(|error| error.to_string())?;
    let command = command_from_payload(&value);
    let input_text = tool_input_text(&value);
    let tool_input = tool_activity_input(&value);
    let tool_name = value_string(&value, &["tool_name", "toolName", "tool"]);
    let skill = value_string(&value, &["skill", "skill_name", "skillName"]).unwrap_or_default();
    let project = overrides
        .project
        .clone()
        .or_else(|| value_string(&value, &["cwd", "project", "projectPath"]))
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
    let project = project_root_for_path(&project);
    let session_id = overrides
        .session
        .clone()
        .or_else(|| value_string(&value, &["session_id", "sessionId", "session"]))
        .unwrap_or_else(|| "unknown-session".into());
    let task_id = overrides
        .task
        .clone()
        .or_else(|| {
            value_string(
                &value,
                &["task_id", "taskId", "current_task", "currentTask"],
            )
            .map(normalize_task_id)
        })
        .or_else(|| task_dir_from_payload(&project, &command))
        .or_else(|| task_dir_from_payload(&project, &input_text))
        .or_else(|| extract_task_arg(&command, tool_name.as_deref()))
        .or_else(|| runtime_task_for_session(&project, &session_id))
        /* CLI fallback：session pointer 缺失时读 .current-task（不伪造活跃时间） */
        .or_else(|| current_task_fallback(&project))
        .or_else(|| runtime_task_for_project(&project))
        .or_else(|| task_dir_from_payload(&project, input));
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_i64)
        .unwrap_or_else(now_seconds);
    let event_name = overrides
        .event
        .clone()
        .or_else(|| {
            value_string(
                &value,
                &["hook_event_name", "hookEventName", "event_name", "event"],
            )
        })
        .unwrap_or_else(|| "SessionStart".into());
    let activity = if !command.is_empty() {
        Some(command.clone())
    } else if !input_text.is_empty() {
        Some(input_text.clone())
    } else {
        event_activity(&value, tool_name.as_deref())
    };
    Ok(HookEvent {
        session_id,
        agent_kind: overrides
            .agent
            .clone()
            .or_else(|| value_string(&value, &["agent", "agent_kind", "agentKind"]))
            .unwrap_or_else(|| "unknown".into()),
        project,
        task_id,
        event_name,
        tool_name: tool_name.clone(),
        tool_input,
        activity,
        action: classify_tool(tool_name.as_deref().unwrap_or_default(), &command, &skill),
        timestamp,
    })
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn parse_overrides(args: impl Iterator<Item = String>) -> HookOverrides {
    let mut out = HookOverrides::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let value = args.next();
        match arg.as_str() {
            "--agent" => out.agent = value,
            "--event" => out.event = value,
            "--session" => out.session = value,
            "--project" => out.project = value,
            "--task" => out.task = value,
            _ => {}
        }
    }
    out
}

pub fn run_hook_cli() {
    let overrides = parse_overrides(std::env::args().skip(2));
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let input = if input.trim().is_empty() {
        "{}"
    } else {
        input.as_str()
    };
    let event = match parse_hook_payload(input, &overrides) {
        Ok(event) => event,
        Err(error) => {
            eprintln!("[hook] invalid payload: {error}");
            return;
        }
    };
    let socket = config::socket_path();
    let queue = config::inbox_dir();
    if !ipc::send_event(&event, &socket, &queue) {
        eprintln!("[hook] unable to deliver event");
    }
    /* Cursor 的 command hook 通过 stdout 返回 JSON。输出 {}（fail-open）明确表示
    不阻断任何命令——即使事件送达失败也不影响 Cursor 的权限/命令行为。
    Claude/Codex 不期望 stdout JSON，保持不输出。 */
    if overrides.agent.as_deref() == Some("cursor") {
        println!("{{}}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::TrellisAction;

    #[test]
    fn decodes_hook_payload_from_stdin_shape() {
        let payload = r#"{
            "session_id":"s1",
            "cwd":"/repo",
            "hook_event_name":"PreToolUse",
            "tool_name":"trellis-implement",
            "tool_input":{"command":"trellis-implement 07-demo"}
        }"#;
        let event = parse_hook_payload(payload, &HookOverrides::default()).unwrap();
        assert_eq!(event.session_id, "s1");
        assert_eq!(event.project, "/repo");
        assert_eq!(event.event_name, "PreToolUse");
        assert_eq!(event.action, Some(TrellisAction::Implement));
    }

    #[test]
    fn explicit_overrides_win_over_payload() {
        let overrides = HookOverrides {
            agent: Some("codex".into()),
            event: Some("Stop".into()),
            session: Some("override".into()),
            project: Some("/work".into()),
            task: Some("07-demo".into()),
        };
        let event =
            parse_hook_payload(r#"{"session_id":"payload","cwd":"/repo"}"#, &overrides).unwrap();
        assert_eq!(event.agent_kind, "codex");
        assert_eq!(event.event_name, "Stop");
        assert_eq!(event.session_id, "override");
        assert_eq!(event.project, "/work");
        assert_eq!(event.task_id.as_deref(), Some("07-demo"));
    }

    #[test]
    fn current_task_path_is_reduced_to_task_directory() {
        let event = parse_hook_payload(
            r#"{"cwd":"/repo","current_task":".trellis/tasks/07-demo","hook_event_name":"PreToolUse"}"#,
            &HookOverrides::default(),
        )
        .unwrap();
        assert_eq!(event.task_id.as_deref(), Some("07-demo"));
    }

    #[test]
    fn trellis_tool_argument_identifies_bare_task_id() {
        let event = parse_hook_payload(
            r#"{"cwd":"/repo","tool_name":"trellis-implement","tool_input":{"command":"trellis-implement 07-demo"}}"#,
            &HookOverrides::default(),
        )
        .unwrap();
        assert_eq!(event.task_id.as_deref(), Some("07-demo"));
    }

    #[test]
    fn task_script_argument_identifies_bare_task_id() {
        let event = parse_hook_payload(
            r#"{"cwd":"/repo","tool_name":"Bash","tool_input":{"command":"python3 .trellis/scripts/task.py start 07-demo"}}"#,
            &HookOverrides::default(),
        )
        .unwrap();
        assert_eq!(event.task_id.as_deref(), Some("07-demo"));
    }

    #[test]
    fn read_tool_path_binds_task_and_exposes_activity() {
        let project = temp_project("read-tool");
        let payload = format!(
            r#"{{"cwd":"{}","session_id":"s-read","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{{"file_path":"{}/.trellis/tasks/07-demo/task.json"}}}}"#,
            jpath(&project),
            jpath(&project)
        );
        let event = parse_hook_payload(&payload, &HookOverrides::default()).unwrap();
        assert_eq!(event.task_id.as_deref(), Some("07-demo"));
        let expected_activity = format!("{}/.trellis/tasks/07-demo/task.json", project.display());
        assert_eq!(event.activity.as_deref(), Some(expected_activity.as_str()));
        assert_eq!(
            event
                .tool_input
                .as_ref()
                .and_then(|input| input.file_path.as_deref()),
            Some(expected_activity.as_str())
        );
        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn prompt_and_session_events_have_visible_activity() {
        let prompt = parse_hook_payload(
            r#"{"cwd":"/repo","hook_event_name":"UserPromptSubmit","prompt":"用 trellis 规划任务"}"#,
            &HookOverrides::default(),
        )
        .unwrap();
        assert_eq!(prompt.activity.as_deref(), Some("用 trellis 规划任务"));

        let start = parse_hook_payload(
            r#"{"cwd":"/repo","hook_event_name":"SessionStart"}"#,
            &HookOverrides::default(),
        )
        .unwrap();
        assert_eq!(start.activity, None);
    }

    #[test]
    fn nested_cwd_is_normalized_to_trellis_project_root() {
        let project = temp_project("nested-cwd");
        let nested = project.join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        let payload = format!(
            r#"{{"cwd":"{}","tool_name":"trellis-implement","tool_input":{{"command":"trellis-implement 07-demo"}}}}"#,
            jpath(&nested)
        );
        let event = parse_hook_payload(&payload, &HookOverrides::default()).unwrap();
        assert_eq!(
            event.project,
            crate::platform::normalize_path(&project).to_string_lossy()
        );
        assert_eq!(event.task_id.as_deref(), Some("07-demo"));
        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn active_runtime_session_identifies_task_without_payload_task_id() {
        let project =
            std::env::temp_dir().join(format!("trellis-card-hook-session-{}", std::process::id()));
        let sessions = project.join(".trellis/.runtime/sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join("codex_session-1.json"),
            r#"{"current_task":".trellis/tasks/07-demo"}"#,
        )
        .unwrap();
        let payload = format!(
            r#"{{"cwd":"{}","session_id":"session-1","hook_event_name":"PreToolUse"}}"#,
            jpath(&project)
        );
        let event = parse_hook_payload(&payload, &HookOverrides::default()).unwrap();
        assert_eq!(event.task_id.as_deref(), Some("07-demo"));
        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn newest_project_runtime_session_binds_when_hook_session_id_is_missing() {
        let project = temp_project("project-runtime-fallback");
        let sessions = project.join(".trellis/.runtime/sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join("claude-known.json"),
            r#"{"last_seen_at":"2026-08-02T12:11:01Z","current_task":".trellis/tasks/07-demo"}"#,
        )
        .unwrap();
        let payload = format!(
            r#"{{"cwd":"{}","session_id":"rotated-session","hook_event_name":"PreToolUse"}}"#,
            jpath(&project)
        );
        let event = parse_hook_payload(&payload, &HookOverrides::default()).unwrap();
        assert_eq!(event.task_id.as_deref(), Some("07-demo"));
        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn unrelated_task_path_in_command_does_not_bind_session() {
        let project = std::env::temp_dir().join(format!(
            "trellis-card-hook-payload-task-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(project.join(".trellis/tasks/07-local")).unwrap();
        let payload = format!(
            r#"{{"cwd":"{}","session_id":"session-2","hook_event_name":"PreToolUse","tool_input":{{"command":"cat /other/.trellis/tasks/07-other/prd.md"}}}}"#,
            jpath(&project)
        );
        let event = parse_hook_payload(&payload, &HookOverrides::default()).unwrap();
        assert_eq!(event.task_id, None);
        let _ = std::fs::remove_dir_all(project);
    }

    /* ---- .current-task CLI fallback（P0-2） ---- */

    fn temp_project(name: &str) -> std::path::PathBuf {
        let project =
            std::env::temp_dir().join(format!("trellis-card-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(project.join(".trellis/tasks/07-demo")).unwrap();
        project
    }

    // 测试里把路径内插进 JSON payload 时，Windows 路径含反斜杠（\U、\T 等）会让
    // serde_json 报 "invalid escape"。统一转义为合法的 \\。
    fn jpath(path: &std::path::Path) -> String {
        path.display().to_string().replace('\\', "\\\\")
    }

    #[test]
    fn current_task_path_pointer_falls_back_when_no_session() {
        let project = temp_project("ct-path");
        std::fs::write(
            project.join(".trellis/.current-task"),
            ".trellis/tasks/07-demo\n",
        )
        .unwrap();
        let payload = format!(
            r#"{{"cwd":"{}","session_id":"unknown-session","hook_event_name":"SessionStart"}}"#,
            jpath(&project)
        );
        let event = parse_hook_payload(&payload, &HookOverrides::default()).unwrap();
        assert_eq!(event.task_id.as_deref(), Some("07-demo"));
        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn current_task_bare_id_falls_back_when_no_session() {
        let project = temp_project("ct-bare");
        std::fs::write(project.join(".trellis/.current-task"), "07-demo").unwrap();
        let payload = format!(
            r#"{{"cwd":"{}","session_id":"unknown-session","hook_event_name":"SessionStart"}}"#,
            jpath(&project)
        );
        let event = parse_hook_payload(&payload, &HookOverrides::default()).unwrap();
        assert_eq!(event.task_id.as_deref(), Some("07-demo"));
        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn current_task_missing_file_yields_none() {
        let project = temp_project("ct-missing");
        let payload = format!(
            r#"{{"cwd":"{}","session_id":"unknown-session","hook_event_name":"SessionStart"}}"#,
            jpath(&project)
        );
        let event = parse_hook_payload(&payload, &HookOverrides::default()).unwrap();
        assert_eq!(event.task_id, None);
        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn current_task_pointing_to_nonexistent_dir_yields_none() {
        let project = temp_project("ct-ghost");
        std::fs::write(project.join(".trellis/.current-task"), "07-ghost").unwrap();
        let payload = format!(
            r#"{{"cwd":"{}","session_id":"unknown-session","hook_event_name":"SessionStart"}}"#,
            jpath(&project)
        );
        let event = parse_hook_payload(&payload, &HookOverrides::default()).unwrap();
        assert_eq!(event.task_id, None);
        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn session_pointer_takes_priority_over_current_task() {
        let project = temp_project("ct-priority");
        /* session 指针指向 07-demo，.current-task 指向 07-session（不存在也不重要，session 应优先） */
        std::fs::create_dir_all(project.join(".trellis/.runtime/sessions")).unwrap();
        std::fs::write(
            project.join(".trellis/.runtime/sessions/session-9.json"),
            r#"{"current_task":".trellis/tasks/07-demo"}"#,
        )
        .unwrap();
        std::fs::write(project.join(".trellis/.current-task"), "07-demo").unwrap();
        let payload = format!(
            r#"{{"cwd":"{}","session_id":"session-9","hook_event_name":"SessionStart"}}"#,
            jpath(&project)
        );
        let event = parse_hook_payload(&payload, &HookOverrides::default()).unwrap();
        assert_eq!(event.task_id.as_deref(), Some("07-demo"));
        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn payload_task_takes_priority_over_current_task() {
        let project = temp_project("ct-payload");
        std::fs::create_dir_all(project.join(".trellis/tasks/08-other")).unwrap();
        std::fs::write(project.join(".trellis/.current-task"), "07-demo").unwrap();
        let payload = format!(
            r#"{{"cwd":"{}","session_id":"unknown-session","hook_event_name":"PreToolUse","tool_input":{{"command":"cat {}/.trellis/tasks/08-other/prd.md"}}}}"#,
            jpath(&project),
            jpath(&project)
        );
        let event = parse_hook_payload(&payload, &HookOverrides::default()).unwrap();
        assert_eq!(event.task_id.as_deref(), Some("08-other"));
        let _ = std::fs::remove_dir_all(project);
    }

    /* ---- Cursor hooks 解析（扁平 payload，字段名与官方文档一致） ---- */

    #[test]
    fn cursor_session_start_payload_parses_without_cwd() {
        /* Cursor sessionStart 输入只有 session_id/composer_mode，无 cwd；
        project 走 override（Cursor hook 命令带 --project）。 */
        let overrides = HookOverrides {
            agent: Some("cursor".into()),
            project: Some("/repo".into()),
            ..HookOverrides::default()
        };
        let event = parse_hook_payload(
            r#"{"hook_event_name":"sessionStart","session_id":"conv-123","composer_mode":"agent"}"#,
            &overrides,
        )
        .unwrap();
        assert_eq!(event.event_name, "sessionStart");
        assert_eq!(event.session_id, "conv-123");
        assert_eq!(event.agent_kind, "cursor");
        assert_eq!(event.project, "/repo");
    }

    #[test]
    fn cursor_pre_tool_use_payload_parses() {
        /* Cursor preToolUse 输入：tool_name/tool_input.command/cwd。 */
        let event = parse_hook_payload(
            r#"{"hook_event_name":"preToolUse","tool_name":"Shell","tool_input":{"command":"cargo test","working_directory":"/repo"},"cwd":"/repo","tool_use_id":"abc"}"#,
            &HookOverrides::default(),
        )
        .unwrap();
        assert_eq!(event.event_name, "preToolUse");
        assert_eq!(event.tool_name.as_deref(), Some("Shell"));
        assert_eq!(event.activity.as_deref(), Some("cargo test"));
        assert_eq!(event.project, "/repo");
    }

    #[test]
    fn cursor_before_shell_execution_payload_parses() {
        /* Cursor beforeShellExecution 输入：command/cwd，无 tool_name。 */
        let event = parse_hook_payload(
            r#"{"hook_event_name":"beforeShellExecution","command":"git status","cwd":"/repo","sandbox":false}"#,
            &HookOverrides::default(),
        )
        .unwrap();
        assert_eq!(event.event_name, "beforeShellExecution");
        assert_eq!(event.activity.as_deref(), Some("git status"));
        assert_eq!(event.project, "/repo");
    }

    #[test]
    fn cursor_stop_payload_parses() {
        /* Cursor stop 输入：status/loop_count。 */
        let event = parse_hook_payload(
            r#"{"hook_event_name":"stop","status":"completed","loop_count":0,"cwd":"/repo"}"#,
            &HookOverrides::default(),
        )
        .unwrap();
        assert_eq!(event.event_name, "stop");
    }

    #[test]
    fn cursor_session_end_payload_parses() {
        let event = parse_hook_payload(
            r#"{"hook_event_name":"sessionEnd","session_id":"conv-1","reason":"completed","duration_ms":45000,"cwd":"/repo"}"#,
            &HookOverrides::default(),
        )
        .unwrap();
        assert_eq!(event.event_name, "sessionEnd");
        assert_eq!(event.session_id, "conv-1");
    }
}
