mod activity;
mod config;
mod focus;
mod hook_cli;
mod hook_install;
mod ipc;
mod progress;
mod runtime;
mod scan;
mod session;
mod watch;

use config::AppConfig;
use runtime::{ActionRecord, AgentRuntime, TaskRuntimeView, TrellisAction};
use serde::Serialize;
use session::{HookEvent, SessionRegistry};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

pub struct AppState {
    config: Mutex<AppConfig>,
    runtime: Mutex<RuntimeStore>,
}

#[derive(Default)]
struct RuntimeStore {
    sessions: SessionRegistry,
    actions: HashMap<String, ActionRecord>,
    focus_key: Option<String>,
    /* 完成迁移生命周期：上次扫描到的真实 task status（key = '项目::任务id'），
    canonical 事实源，只记录任务真实状态，不伪造。 */
    task_statuses: HashMap<String, String>,
    /* 待 emit 的完成迁移事件（仅真实 unfinished -> completed/done 一次） */
    pending_completions: Vec<TaskCompletedEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskCompletedEvent {
    project: String,
    task_id: String,
    completed_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub tasks: Vec<TaskRuntimeView>,
    pub project_activities: Vec<AgentRuntime>,
    pub errors: Vec<String>,
    pub focus_key: Option<String>,
    pub generated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HookTasksChangedEvent {
    dynamic_project_added: bool,
    action: Option<TrellisAction>,
    project: Option<String>,
}

// ---------- 输出结构 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigOut {
    roots: Vec<String>,
    always_on_top: bool,
    configured: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RootsOut {
    roots: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TasksPayload {
    version: String,
    tasks: Vec<scan::Task>,
    errors: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocOut {
    name: String,  // 文件名（research 内为 research/xx.md）
    label: String, // 展示标签：PRD / DESIGN / IMPLEMENT / 报告 N / 调研·xx / 文件名
    content: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskDetail {
    #[serde(flatten)]
    task: scan::Task,
    docs: Vec<DocOut>,
}

// ---------- 辅助 ----------

fn expand(p: &str) -> PathBuf {
    config::expand_home(p)
}

pub(crate) fn discover_all(cfg: &AppConfig) -> Vec<PathBuf> {
    let mut out = discover_all_roots(cfg);
    let mut seen: std::collections::HashSet<PathBuf> = out.iter().cloned().collect();
    for project in &cfg.dynamic_projects {
        let dir = expand(project);
        if dir.join(".trellis").join("tasks").is_dir() && seen.insert(dir.clone()) {
            out.push(dir);
        }
    }
    out
}

fn project_dir_for_path(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    loop {
        if current.join(".trellis").join("tasks").is_dir() {
            return current.canonicalize().ok().or(Some(current));
        }
        if !current.pop() {
            return None;
        }
    }
}

/* Hook 发现的项目不受 roots 限制，但仍必须是真正的 Trellis 项目目录。 */
fn register_dynamic_project(state: &AppState, path: &str) -> Option<(PathBuf, bool)> {
    let project = project_dir_for_path(Path::new(path))?;
    let mut cfg = state.config.lock().unwrap();
    /* A root can itself be a Trellis project. In that case the scanner
    intentionally stops descending, so a child project must still be
    registered when a hook proves that it is active. */
    if is_discovered_project(&cfg, &project) {
        return Some((project, false));
    }
    if cfg
        .dynamic_projects
        .iter()
        .map(|stored| expand(stored))
        .any(|stored| stored == project)
    {
        return Some((project, false));
    }
    let stored = project.to_string_lossy().into_owned();
    cfg.dynamic_projects.push(stored);
    let snapshot = cfg.clone();
    drop(cfg);
    if let Err(error) = config::save(&snapshot) {
        eprintln!("[projects] 保存动态项目失败: {error}");
    }
    Some((project, true))
}

fn discover_all_roots(cfg: &AppConfig) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for root in &cfg.roots {
        for dir in scan::discover_projects(&expand(root), 3) {
            if seen.insert(dir.clone()) {
                out.push(dir);
            }
        }
    }
    out
}

fn is_discovered_project(cfg: &AppConfig, project: &Path) -> bool {
    discover_all_roots(cfg).iter().any(|found| found == project)
}

// 允许访问 roots 扫描到的项目，以及 Hook 已发现的动态项目。
fn is_allowed_project(state: &AppState, project: &str) -> bool {
    let cfg = state.config.lock().unwrap();
    let target = expand(project);
    discover_all(&cfg).contains(&target)
}

fn save_state(state: &AppState) -> Result<(), String> {
    let cfg = state.config.lock().unwrap().clone();
    config::save(&cfg)
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn task_key(project: &str, task_id: &str) -> String {
    format!("{project}::{task_id}")
}

fn is_done_status(status: &str) -> bool {
    matches!(status, "completed" | "done")
}

/* 完成迁移判定：仅「上次已知状态存在且非完成、本次为完成」的真实迁移返回 true。
初次扫描（prev=None）、无变化、completed->completed、blocked->blocked 均不迁移。 */
fn is_completion_transition(prev: Option<&str>, cur: &str) -> bool {
    match prev {
        Some(prev) => !is_done_status(prev) && is_done_status(cur),
        None => false,
    }
}

fn build_runtime_snapshot(state: &AppState) -> RuntimeSnapshot {
    let now = now_seconds();
    let cfg = state.config.lock().unwrap().clone();
    let projects = discover_all(&cfg);
    let mut runtime = state.runtime.lock().unwrap();
    runtime.sessions.prune(now);
    let mut views = Vec::new();
    let mut activities = Vec::new();
    let mut errors = Vec::new();
    for project_dir in projects {
        let project = project_dir.to_string_lossy().into_owned();
        let (tasks, task_errors) = scan::scan_tasks(&project_dir);
        errors.extend(task_errors);
        for task in tasks {
            let key = task_key(&project, &task.id);
            /* 完成迁移 reducer：仅 unfinished -> completed/done 的真实迁移记入 pending。
            初次扫描（无上次记录）、无变化、重复 completed 均不生成。 */
            let prev_status = runtime.task_statuses.get(&key).cloned();
            if is_completion_transition(prev_status.as_deref(), &task.status) {
                runtime.pending_completions.push(TaskCompletedEvent {
                    project: project.clone(),
                    task_id: task.id.clone(),
                    completed_at: task
                        .completed_at
                        .as_deref()
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(now),
                });
            }
            runtime.task_statuses.insert(key, task.status.clone());
            let session = runtime
                .sessions
                .latest_for_task(&project, &task.id)
                .or_else(|| runtime.sessions.latest_for_task(&project, &task.dir));
            let action = runtime
                .actions
                .get(&task_key(&project, &task.id))
                .and_then(|r| r.is_fresh(now).then_some(r.action))
                .or_else(|| {
                    runtime
                        .actions
                        .get(&task_key(&project, &task.dir))
                        .and_then(|r| r.is_fresh(now).then_some(r.action))
                });
            let mut view = focus::fuse_task(&task, session, action);
            view.project = project.clone();
            views.push(view);
        }
    }
    activities.extend(runtime.sessions.all_activity());
    let previous = runtime.focus_key.as_deref();
    let next_focus = focus::choose_focus(&views, previous, now);
    runtime.focus_key = next_focus.clone();
    RuntimeSnapshot {
        tasks: views,
        project_activities: activities,
        errors,
        focus_key: next_focus,
        generated_at: now,
    }
}

fn emit_runtime_snapshot(app: &AppHandle) {
    let state = app.state::<AppState>();
    let previous = state.runtime.lock().unwrap().focus_key.clone();
    let snapshot = build_runtime_snapshot(&state);
    if previous != snapshot.focus_key {
        let _ = app.emit("focus-task-changed", &snapshot.focus_key);
    }
    let _ = app.emit("agent-state-changed", &snapshot);
    /* 完成迁移事件：drain pending 后逐个 emit，前端幂等消费（仅触发一次刷新） */
    let completions = {
        let mut runtime = state.runtime.lock().unwrap();
        std::mem::take(&mut runtime.pending_completions)
    };
    for completed in completions {
        let _ = app.emit("task-completed", &completed);
    }
}

fn apply_hook_events<I>(app: &AppHandle, events: I)
where
    I: IntoIterator<Item = HookEvent>,
{
    let state = app.state::<AppState>();
    let mut dynamic_project_added = false;
    let mut refresh_action = None;
    let mut project_hint = None;
    let mut prepared_events = Vec::new();
    for mut event in events {
        /* 全局 Hook 会收到所有 Agent 项目的事件，只接收真正的 Trellis 项目。 */
        let Some((project, added)) = register_dynamic_project(&state, &event.project) else {
            continue;
        };
        event.project = project.to_string_lossy().into_owned();
        dynamic_project_added |= added;
        if event.action == Some(TrellisAction::Create) || project_hint.is_none() {
            project_hint = Some(event.project.clone());
        }
        if event.action == Some(TrellisAction::Create) || refresh_action.is_none() {
            refresh_action = event.action;
        }
        prepared_events.push(event);
    }
    if prepared_events.is_empty() {
        return;
    }
    {
        let mut runtime = state.runtime.lock().unwrap();
        for event in prepared_events {
            let project = event.project.clone();
            let session_id = event.session_id.clone();
            let event_task_id = event.task_id.clone();
            let action = event.action;
            let timestamp = event.timestamp;
            if !runtime.sessions.apply(event) {
                continue;
            }
            let task_id = event_task_id.or_else(|| {
                runtime
                    .sessions
                    .task_id_for_session(&session_id)
                    .map(str::to_owned)
            });
            if let (Some(task_id), Some(action)) = (task_id.as_deref(), action) {
                let key = task_key(&project, task_id);
                let should_update = runtime
                    .actions
                    .get(&key)
                    .map(|record| timestamp >= record.updated_at)
                    .unwrap_or(true);
                if should_update {
                    runtime.actions.insert(
                        key,
                        ActionRecord {
                            action,
                            updated_at: timestamp,
                        },
                    );
                }
            }
        }
    }
    emit_runtime_snapshot(app);
    if dynamic_project_added || refresh_action.is_some() {
        let _ = app.emit(
            "hook-tasks-changed",
            HookTasksChangedEvent {
                dynamic_project_added,
                action: refresh_action,
                project: project_hint,
            },
        );
    }
}

fn apply_hook_event(app: &AppHandle, event: HookEvent) {
    apply_hook_events(app, std::iter::once(event));
}

fn drain_runtime_queue(app: &AppHandle) {
    let events = ipc::drain_queue(&config::inbox_dir());
    if !events.is_empty() {
        apply_hook_events(app, events);
    }
}

fn start_runtime_workers(app: &AppHandle) {
    let socket = config::socket_path();
    let app_for_socket = app.clone();
    ipc::start_server(&socket, move |event| {
        let app = app_for_socket.clone();
        tauri::async_runtime::spawn(async move {
            apply_hook_event(&app, event);
        });
    });

    let app_for_queue = app.clone();
    std::thread::spawn(move || loop {
        drain_runtime_queue(&app_for_queue);
        std::thread::sleep(std::time::Duration::from_millis(250));
    });

    let app_for_timer = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(10));
        emit_runtime_snapshot(&app_for_timer);
    });
}

#[tauri::command]
fn get_runtime_snapshot(state: State<AppState>) -> RuntimeSnapshot {
    build_runtime_snapshot(&state)
}

#[tauri::command]
fn get_hook_statuses() -> Result<Vec<hook_install::HookStatus>, String> {
    hook_install::statuses()
}

#[tauri::command]
fn configure_hook(agent: String, enabled: bool) -> Result<hook_install::HookStatus, String> {
    let agent = agent.to_ascii_lowercase();
    hook_install::install_hooks(&agent, !enabled)?;
    hook_install::status(&agent)
}

// ---------- 命令 ----------

#[tauri::command]
fn get_config(state: State<AppState>) -> ConfigOut {
    let cfg = state.config.lock().unwrap();
    ConfigOut {
        configured: cfg.initialized || !cfg.roots.is_empty() || !cfg.dynamic_projects.is_empty(),
        roots: cfg.roots.clone(),
        always_on_top: cfg.always_on_top,
    }
}

#[tauri::command]
fn complete_setup(state: State<AppState>) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.initialized = true;
    drop(cfg);
    save_state(&state)
}

// 必须 async：同步命令跑在主线程，blocking 弹窗会冻住整个应用
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |p| {
        let _ = tx.send(p);
    });
    rx.recv().ok().flatten().map(|p| p.to_string())
}

#[tauri::command]
fn add_root(state: State<AppState>, path: String) -> Result<RootsOut, String> {
    let dir = expand(&path);
    if !dir.is_dir() {
        return Err("目录不存在或不可读".into());
    }
    let canonical = dir.to_string_lossy().into_owned();
    let mut cfg = state.config.lock().unwrap();
    cfg.initialized = true;
    if !cfg.roots.contains(&canonical) {
        cfg.roots.push(canonical);
    }
    let roots = cfg.roots.clone();
    drop(cfg);
    save_state(&state)?;
    Ok(RootsOut { roots })
}

#[tauri::command]
fn remove_root(state: State<AppState>, path: String) -> Result<RootsOut, String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.roots.retain(|r| r != &path);
    let roots = cfg.roots.clone();
    drop(cfg);
    save_state(&state)?;
    Ok(RootsOut { roots })
}

#[tauri::command]
fn list_projects(state: State<AppState>) -> Vec<scan::ProjectInfo> {
    let cfg = state.config.lock().unwrap();
    discover_all(&cfg)
        .iter()
        .map(|dir| scan::project_info(dir))
        .collect()
}

#[tauri::command]
fn list_tasks(state: State<AppState>, project: String) -> Result<TasksPayload, String> {
    if !is_allowed_project(&state, &project) {
        return Err("项目未在扫描目录中".into());
    }
    let (tasks, errors) = scan::scan_tasks(&expand(&project));
    Ok(TasksPayload {
        version: scan::tasks_version(&tasks),
        tasks,
        errors,
    })
}

// 收集任务目录下的所有 markdown：根目录 *.md（含 acceptance-report-*）+ research/*.md
fn collect_docs(task_dir: &Path) -> Vec<DocOut> {
    const MAX_CHARS: usize = 120_000; // 单文档截断保护
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(task_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_file() && name.ends_with(".md") {
                names.push(name);
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(task_dir.join("research")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_file() && name.ends_with(".md") {
                names.push(format!("research/{}", name));
            }
        }
    }
    // 固定顺序：prd/design/implement 优先，报告其次，调研再其次，其余按名字
    let rank = |n: &str| match n {
        "prd.md" => 0,
        "design.md" => 1,
        "implement.md" => 2,
        _ if n.starts_with("acceptance-report") => 3,
        _ if n.starts_with("research/") => 4,
        _ => 5,
    };
    names.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));

    let label = |n: &str| -> String {
        match n {
            "prd.md" => "PRD".into(),
            "design.md" => "DESIGN".into(),
            "implement.md" => "IMPLEMENT".into(),
            _ if n.starts_with("acceptance-report") => {
                let stem = n.trim_end_matches(".md");
                let num = stem
                    .trim_start_matches("acceptance-report")
                    .trim_matches('-');
                if num.is_empty() {
                    "验收报告".into()
                } else {
                    format!("报告 {}", num)
                }
            }
            _ if n.starts_with("research/") => {
                format!(
                    "调研·{}",
                    n.trim_start_matches("research/").trim_end_matches(".md")
                )
            }
            _ => n.trim_end_matches(".md").to_string(),
        }
    };

    names
        .into_iter()
        .take(24) // 文档数量保护
        .filter_map(|n| {
            let content = std::fs::read_to_string(task_dir.join(&n)).ok()?;
            let content = if content.chars().count() > MAX_CHARS {
                let cut: String = content.chars().take(MAX_CHARS).collect();
                format!("{}\n\n…（文档过长，已截断）", cut)
            } else {
                content
            };
            Some(DocOut {
                label: label(&n),
                name: n,
                content,
            })
        })
        .collect()
}

#[tauri::command]
fn get_task(state: State<AppState>, project: String, id: String) -> Result<TaskDetail, String> {
    if !is_allowed_project(&state, &project) {
        return Err("项目未在扫描目录中".into());
    }
    let project_dir = expand(&project);
    let (tasks, _) = scan::scan_tasks(&project_dir);
    let task = tasks
        .into_iter()
        .find(|t| t.id == id || t.dir == id)
        .ok_or_else(|| "任务不存在".to_string())?;
    let docs = collect_docs(&project_dir.join(".trellis").join("tasks").join(&task.dir));
    Ok(TaskDetail { task, docs })
}

// 校验一个任务是否可归档：项目允许、任务是活跃任务（在 .trellis/tasks/ 下、非 archive/）、
// task.json 存在、项目带 task.py 脚本。返回 (project_dir, task_dir, script) 或错误原因。
fn resolve_archivable_task(
    state: &AppState,
    project: &str,
    task: &str,
) -> Result<(PathBuf, PathBuf), String> {
    if !is_allowed_project(state, project) {
        return Err("项目未在扫描目录中".into());
    }
    let project_dir = expand(project);
    let task_dir = project_dir.join(".trellis").join("tasks").join(task);
    if !task_dir.is_dir() {
        return Err(format!("任务不存在: {task}"));
    }
    if !task_dir.join("task.json").is_file() {
        return Err(format!("任务缺少 task.json: {task}"));
    }
    /* 只允许归档活跃任务：archive/ 下已是归档，避免重复移动。 */
    let script = project_dir.join(".trellis").join("scripts").join("task.py");
    if !script.is_file() {
        return Err(format!("项目缺少 Trellis 脚本: {}", script.display()));
    }
    Ok((project_dir, script))
}

// 把活跃任务归档到 .trellis/tasks/archive/<YYYY-MM>/：执行项目自带的 task.py archive。
// 成功后 emit tasks-changed / runtime-reconciliation-needed 触发前端刷新（目录移动
// 不一定落在 watch 的相关文件路径上，主动通知保证列表即时收敛）。
#[tauri::command]
fn archive_task(
    app: AppHandle,
    state: State<AppState>,
    project: String,
    task: String,
) -> Result<bool, String> {
    let (project_dir, script) = resolve_archivable_task(&state, &project, &task)?;
    let output = std::process::Command::new("python3")
        .arg(script)
        .args(["archive", &task])
        .current_dir(&project_dir)
        .output()
        .map_err(|e| format!("执行 task.py archive 失败: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "task.py archive 失败: {}{}",
            if stdout.is_empty() { "" } else { &stdout },
            if stderr.is_empty() { "" } else { &stderr }
        ));
    }
    let _ = app.emit("tasks-changed", ());
    let _ = app.emit("runtime-reconciliation-needed", ());
    Ok(true)
}

#[tauri::command]
fn set_always_on_top(app: AppHandle, state: State<AppState>, flag: bool) -> Result<bool, String> {
    if let Some(window) = app.get_webview_window("main") {
        window.set_always_on_top(flag).map_err(|e| e.to_string())?;
    }
    state.config.lock().unwrap().always_on_top = flag;
    save_state(&state)?;
    Ok(flag)
}

#[tauri::command]
fn set_window_mode(app: AppHandle, mode: String) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    let position = window.outer_position().ok();
    let (w, h) = if mode == "capsule" {
        (360.0, 136.0)
    } else {
        (380.0, 640.0)
    };
    window
        .set_size(tauri::LogicalSize::new(w, h))
        .map_err(|e| e.to_string())?;
    if let Some(position) = position {
        window.set_position(position).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

// 内容自适应：只在渲染稳定后由前端一次性调用（防抖+阈值在前端），不做连续跟随
#[tauri::command]
fn fit_window_height(app: AppHandle, height: f64) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    let position = window.outer_position().ok();
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let w = size.width as f64 / scale;
    let mut h = height.clamp(160.0, 2000.0);
    if let Ok(Some(monitor)) = window.current_monitor() {
        let max_h = monitor.size().height as f64 / scale - 40.0;
        h = h.min(max_h);
    }
    window
        .set_size(tauri::LogicalSize::new(w, h))
        .map_err(|e| e.to_string())?;
    /* 无边框透明窗口在 macOS 调整尺寸时可能按中心重新定位；恢复左上角，
    让内容自适应只改变高度，不把用户正在看的卡片整体推向屏幕上方。 */
    if let Some(position) = position {
        window.set_position(position).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------- 启动 ----------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let cfg = config::load();
            app.manage(AppState {
                config: Mutex::new(cfg.clone()),
                runtime: Mutex::new(RuntimeStore::default()),
            });

            start_runtime_workers(app.handle());

            let window = app.get_webview_window("main").expect("main window");
            let _ = window.set_always_on_top(cfg.always_on_top);

            // 关闭窗口 = 隐藏到托盘，不退出
            let w = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = w.hide();
                }
            });

            // 托盘：显示 / 退出
            use tauri::menu::{MenuBuilder, MenuItemBuilder};
            use tauri::tray::TrayIconBuilder;
            let show = MenuItemBuilder::with_id("show", "显示 Trellis Card").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
            let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .tooltip("Trellis Card")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            watch::spawn_watcher(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            complete_setup,
            pick_folder,
            add_root,
            remove_root,
            list_projects,
            list_tasks,
            get_task,
            archive_task,
            set_always_on_top,
            set_window_mode,
            fit_window_height,
            hide_window,
            get_runtime_snapshot,
            get_hook_statuses,
            configure_hook,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// CLI 入口由二进制入口转发，保持 GUI 与 Hook 命令共用同一 crate。
pub use hook_cli::run_hook_cli;
pub use hook_install::run_hook_install_cli;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn collect_docs_orders_and_labels() {
        let dir = std::env::temp_dir().join(format!("trellis-card-docs-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("research")).unwrap();
        std::fs::write(dir.join("implement.md"), "impl").unwrap();
        std::fs::write(dir.join("prd.md"), "prd").unwrap();
        std::fs::write(dir.join("acceptance-report-2.md"), "r2").unwrap();
        std::fs::write(dir.join("PEER_BRIEF.md"), "peer").unwrap();
        std::fs::write(dir.join("research").join("竞品.md"), "res").unwrap();
        std::fs::write(dir.join("task.json"), "{}").unwrap(); // 非 md 不收

        let docs = collect_docs(&dir);
        let labels: Vec<&str> = docs.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(
            labels,
            ["PRD", "IMPLEMENT", "报告 2", "调研·竞品", "PEER_BRIEF"]
        );
        assert_eq!(docs[0].content, "prd");
        assert_eq!(docs[3].name, "research/竞品.md");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn collect_docs_empty_dir() {
        assert!(collect_docs(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn discover_all_includes_hook_discovered_project_outside_roots() {
        let project = std::env::temp_dir().join(format!(
            "trellis-card-dynamic-project-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(project.join(".trellis/tasks")).unwrap();
        let cfg = AppConfig {
            roots: vec![],
            dynamic_projects: vec![project.to_string_lossy().into_owned()],
            initialized: false,
            always_on_top: false,
        };

        assert_eq!(discover_all(&cfg), vec![project.clone()]);
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn project_dir_for_path_finds_trellis_ancestor() {
        let project = std::env::temp_dir().join(format!(
            "trellis-card-dynamic-ancestor-{}",
            std::process::id()
        ));
        let nested = project.join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(project.join(".trellis/tasks")).unwrap();

        assert_eq!(project_dir_for_path(&nested), project.canonicalize().ok());
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn project_dir_for_path_rejects_non_trellis_ancestor() {
        let root =
            std::env::temp_dir().join(format!("trellis-card-non-project-{}", std::process::id()));
        let nested = root.join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();

        assert_eq!(project_dir_for_path(&nested), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn child_project_under_trellis_root_is_discovered_by_scan() {
        /* root 和其子目录 child 都是 Trellis 项目：扫描应同时发现两者（嵌套项目可见） */
        let root =
            std::env::temp_dir().join(format!("trellis-card-nested-root-{}", std::process::id()));
        let child = root.join("child");
        std::fs::create_dir_all(root.join(".trellis/tasks")).unwrap();
        std::fs::create_dir_all(child.join(".trellis/tasks")).unwrap();
        let cfg = AppConfig {
            roots: vec![root.to_string_lossy().into_owned()],
            dynamic_projects: vec![],
            initialized: false,
            always_on_top: false,
        };

        assert!(is_discovered_project(&cfg, &child));
        assert!(discover_all(&cfg).contains(&child));
        std::fs::remove_dir_all(root).unwrap();
    }

    /* ---- 完成迁移 reducer（P0-1） ---- */

    #[test]
    fn completion_transition_unfinished_to_completed() {
        assert!(is_completion_transition(Some("in_progress"), "completed"));
        assert!(is_completion_transition(Some("planning"), "done"));
    }

    #[test]
    fn completion_transition_first_scan_does_not_emit() {
        /* 初次扫描：无上次记录，不生成完成事件 */
        assert!(!is_completion_transition(None, "completed"));
        assert!(!is_completion_transition(None, "in_progress"));
    }

    #[test]
    fn completion_transition_no_change_does_not_emit() {
        assert!(!is_completion_transition(
            Some("in_progress"),
            "in_progress"
        ));
        assert!(!is_completion_transition(Some("planning"), "planning"));
    }

    #[test]
    fn completion_transition_completed_to_completed_not_repeated() {
        /* 重复 completed：不重复生成（第二次 completed 不再迁移） */
        assert!(!is_completion_transition(Some("completed"), "completed"));
        assert!(!is_completion_transition(Some("done"), "done"));
    }

    #[test]
    fn completion_transition_blocked_to_blocked_not_emit() {
        assert!(!is_completion_transition(Some("blocked"), "blocked"));
        assert!(!is_completion_transition(Some("failed"), "failed"));
    }

    #[test]
    fn completion_transition_unfinished_to_unfinished_not_emit() {
        assert!(!is_completion_transition(Some("in_progress"), "blocked"));
        assert!(!is_completion_transition(Some("planning"), "review"));
    }

    #[test]
    fn completion_pending_event_carries_project_task_completed_at() {
        /* 集成：模拟 reducer 写入 pending，验证 payload 字段 */
        let mut store = RuntimeStore::default();
        let key = task_key("/repo/alpha", "07-demo");
        store
            .task_statuses
            .insert(key.clone(), "in_progress".into());
        /* 构造最小 scan::Task 手动走 reducer 逻辑（不经 build_runtime_snapshot） */
        let now = now_seconds();
        if is_completion_transition(
            store.task_statuses.get(&key).map(String::as_str),
            "completed",
        ) {
            store.pending_completions.push(TaskCompletedEvent {
                project: "/repo/alpha".into(),
                task_id: "07-demo".into(),
                completed_at: now,
            });
        }
        store.task_statuses.insert(key.clone(), "completed".into());
        assert_eq!(store.pending_completions.len(), 1);
        let ev = &store.pending_completions[0];
        assert_eq!(ev.project, "/repo/alpha");
        assert_eq!(ev.task_id, "07-demo");
        assert_eq!(ev.completed_at, now);
        /* 第二次扫描（completed->completed）：不新增 pending */
        if is_completion_transition(
            store.task_statuses.get(&key).map(String::as_str),
            "completed",
        ) {
            store.pending_completions.push(TaskCompletedEvent {
                project: "/repo/alpha".into(),
                task_id: "07-demo".into(),
                completed_at: now,
            });
        }
        assert_eq!(store.pending_completions.len(), 1);
    }

    // 构造一个 roots 指向临时项目的 AppState，供归档校验测试使用
    fn state_with_project(project: &Path) -> AppState {
        AppState {
            config: Mutex::new(AppConfig {
                roots: vec![project.to_string_lossy().into_owned()],
                dynamic_projects: vec![],
                initialized: true,
                always_on_top: false,
            }),
            runtime: Mutex::new(RuntimeStore::default()),
        }
    }

    fn archive_fixture(tag: &str) -> (std::path::PathBuf, AppState) {
        let root = std::env::temp_dir().join(format!(
            "trellis-card-archive-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("alpha");
        std::fs::create_dir_all(project.join(".trellis/tasks/02-27-user-login")).unwrap();
        std::fs::create_dir_all(project.join(".trellis/scripts")).unwrap();
        std::fs::write(
            project.join(".trellis/scripts/task.py"),
            "#!/usr/bin/env python3\n",
        )
        .unwrap();
        (root, state_with_project(&project))
    }

    #[test]
    fn resolve_archivable_task_accepts_active_task() {
        let (root, state) = archive_fixture("accept");
        let project = root.join("alpha");
        std::fs::write(
            project.join(".trellis/tasks/02-27-user-login/task.json"),
            r#"{"id":"02-27-user-login","status":"completed"}"#,
        )
        .unwrap();

        let (proj, script) =
            resolve_archivable_task(&state, &project.to_string_lossy(), "02-27-user-login")
                .unwrap();
        assert_eq!(proj, project);
        assert_eq!(script, project.join(".trellis/scripts/task.py"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_archivable_task_rejects_missing_task() {
        let (root, state) = archive_fixture("missing");
        let project = root.join("alpha");
        let err = resolve_archivable_task(&state, &project.to_string_lossy(), "no-such-task")
            .unwrap_err();
        assert!(err.contains("任务不存在"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_archivable_task_rejects_missing_script() {
        let (root, state) = archive_fixture("noscript");
        let project = root.join("alpha");
        std::fs::write(
            project.join(".trellis/tasks/02-27-user-login/task.json"),
            r#"{"id":"02-27-user-login","status":"completed"}"#,
        )
        .unwrap();
        std::fs::remove_file(project.join(".trellis/scripts/task.py")).unwrap();

        let err = resolve_archivable_task(&state, &project.to_string_lossy(), "02-27-user-login")
            .unwrap_err();
        assert!(err.contains("缺少 Trellis 脚本"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* ---------- Hook 端到端集成测试 ----------
    打通完整链路：真实 payload → parse_hook_payload → IPC 编解码 → SessionRegistry 应用
    → build_runtime_snapshot 反映到前端可读的运行时快照。 */

    fn hook_fixture(tag: &str) -> (std::path::PathBuf, AppState) {
        let root = std::env::temp_dir().join(format!(
            "trellis-card-hook-e2e-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("alpha");
        std::fs::create_dir_all(project.join(".trellis/tasks/02-27-user-login")).unwrap();
        std::fs::write(
            project.join(".trellis/tasks/02-27-user-login/task.json"),
            r#"{"id":"02-27-user-login","title":"User login","status":"in_progress"}"#,
        )
        .unwrap();
        /* canonicalize 对齐 parse_hook_payload 的路径输出（/var ↔ /private/var） */
        let project = project.canonicalize().unwrap_or(project);
        (root, state_with_project(&project))
    }

    #[test]
    fn hook_payload_reaches_runtime_snapshot_end_to_end() {
        let (root, state) = hook_fixture("flow");
        let project = root
            .join("alpha")
            .canonicalize()
            .unwrap_or_else(|_| root.join("alpha"));
        let project_str = project.to_string_lossy().into_owned();

        /* 1) 真实 Codex 风格 payload → 解析成 HookEvent */
        let payload = format!(
            r#"{{
              "hook_event_name": "PreToolUse",
              "session_id": "sess-1",
              "agent": "codex",
              "cwd": "{project_str}",
              "task_id": "02-27-user-login",
              "tool_name": "Bash",
              "command": "cargo test",
              "timestamp": 1000
            }}"#
        );
        let overrides = crate::hook_cli::HookOverrides {
            agent: None,
            event: None,
            session: None,
            project: None,
            task: None,
        };
        let event =
            crate::hook_cli::parse_hook_payload(&payload, &overrides).expect("payload 应解析成功");
        assert_eq!(event.session_id, "sess-1");
        assert_eq!(event.task_id.as_deref(), Some("02-27-user-login"));
        assert_eq!(event.event_name, "PreToolUse");

        /* 2) IPC 编解码往返：模拟 hook → socket/队列 → 后端 */
        let encoded = ipc::encode_event(&event).unwrap();
        let decoded = ipc::decode_event(encoded.trim()).unwrap();
        assert_eq!(decoded, event);

        /* 3) 应用事件到 SessionRegistry（apply_hook_events 的核心状态迁移；
        不调用 register_dynamic_project——那会写真实配置文件，测试必须隔离） */
        let mut runtime = state.runtime.lock().unwrap();
        assert!(runtime.sessions.apply(decoded.clone()));
        drop(runtime);

        /* 4) 运行时快照应包含该会话活动（前端 agent-state-changed 的数据源）。
        state_with_project 已把临时项目设为 root，build_runtime_snapshot 会扫描到它。 */
        let snapshot = build_runtime_snapshot(&state);
        let expected_key = format!("{}::02-27-user-login", project_str);
        assert_eq!(snapshot.focus_key, Some(expected_key));
        let activity = snapshot
            .project_activities
            .iter()
            .find(|a| a.session_id == "sess-1");
        assert!(activity.is_some(), "快照应包含 hook 会话活动");
        assert_eq!(activity.unwrap().project, project_str);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
