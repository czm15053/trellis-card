// .trellis 项目发现与 task.json 扫描。
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

const SKIP_DIRS: [&str; 6] = [
    "node_modules",
    ".git",
    "dist",
    "build",
    ".next",
    "__pycache__",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Subtask {
    pub name: String,
    pub status: Option<String>,
}

// task.json 原始结构；老版本任务缺字段按默认处理。
// 注意真实 schema 混用命名：dev_type 是 snake_case，createdAt 是 camelCase。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RawTask {
    pub id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    #[serde(alias = "dev_type")]
    pub dev_type: Option<String>,
    pub scope: Option<String>,
    pub package: Option<String>,
    pub branch: Option<String>,
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub subtasks: Vec<Subtask>,
    pub created_at: Option<String>,
    pub completed_at: Option<String>,
}

// 输出给前端的归一化任务（含计算字段）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub priority: String,
    pub dev_type: Option<String>,
    pub scope: Option<String>,
    pub package: Option<String>,
    pub branch: Option<String>,
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub subtasks: Vec<Subtask>,
    pub created_at: Option<String>,
    pub completed_at: Option<String>,
    pub mtime: i64,
    pub progress: f64,
    pub stage: String,
    pub lane: u8,
    pub partial: f64,
    pub kind: String,
    // true = 该任务位于 .trellis/tasks/archive/ 下（已归档，仅查看）
    pub archived: bool,
    // 指向该任务的 AI 会话（platform + 最近活跃时间）
    pub sessions: Vec<SessionInfo>,
    // prd.md 首段摘要（“在规划/做什么”的实质内容）
    pub excerpt: String,
    // 任务目录产物清单（供翻面背面展示与 phase 推断）
    pub artifacts: Artifacts,
    // 细粒度工作流阶段（比 lane 更精确：规划内部还分 1.0-1.5）
    pub phase: Phase,
    pub dir: String,
}

// 任务目录产物：prd/design/implement 文档、research、jsonl 上下文、验收报告
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Artifacts {
    pub prd: bool,
    pub design: bool,
    pub implement: bool,
    pub research_count: usize,
    // implement.jsonl / check.jsonl 里的真实条目数（不含 _example seed 行）
    pub impl_entries: usize,
    pub check_entries: usize,
    pub report_count: usize,
}

// 细粒度工作流阶段
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Phase {
    pub id: String,
    pub label: String,
    // true = 需要用户注意的警告态（如规范注入未配）
    pub warn: bool,
}

// 统计 jsonl 里带 "file" 键的真实条目（seed 行只有 "_example" 说明，不算）
fn count_jsonl_entries(path: &Path) -> usize {
    fs::read_to_string(path)
        .map(|content| {
            content
                .lines()
                .filter(|line| {
                    let t = line.trim();
                    if t.is_empty() {
                        return false;
                    }
                    serde_json::from_str::<serde_json::Value>(t)
                        .ok()
                        .and_then(|v| v.get("file").cloned())
                        .and_then(|f| f.as_str().map(|s| !s.is_empty()))
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

fn count_md_in_dir(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
                .count()
        })
        .unwrap_or(0)
}

fn scan_artifacts(task_dir: &Path) -> Artifacts {
    let report_count = fs::read_dir(task_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    name.starts_with("acceptance-report") && name.ends_with(".md")
                })
                .count()
        })
        .unwrap_or(0);
    Artifacts {
        prd: task_dir.join("prd.md").is_file(),
        design: task_dir.join("design.md").is_file(),
        implement: task_dir.join("implement.md").is_file(),
        research_count: count_md_in_dir(&task_dir.join("research")),
        impl_entries: count_jsonl_entries(&task_dir.join("implement.jsonl")),
        check_entries: count_jsonl_entries(&task_dir.join("check.jsonl")),
        report_count,
    }
}

// 从 status + artifacts 推断细粒度阶段（对齐 Trellis workflow Phase 1.0-3.5）
fn infer_phase(status: &str, a: &Artifacts) -> Phase {
    let p = |id: &str, label: &str, warn: bool| Phase {
        id: id.into(),
        label: label.into(),
        warn,
    };
    match status {
        "completed" | "done" => p("done", "已完结 · 待归档", false),
        "review" => p("review", "待评审", false),
        "blocked" => p("halt", "卡住了", true),
        "in_progress" => {
            if a.report_count > 0 {
                p("verify", "动手 · 验证阶段", false) // Phase 2.2/3.1
            } else {
                p("implement", "动手 · 实现中", false) // Phase 2.1
            }
        }
        "planning" => {
            if a.impl_entries > 0 {
                p("ready", "规划 1.3 就绪 · 可启动", false) // 规范注入已配
            } else if a.prd {
                p("need-context", "规划中 · 规范注入未配", true) // ★ 最容易漏的一步
            } else {
                p("explore", "规划 · 需求探索中", false) // Phase 1.0-1.1
            }
        }
        _ => p("unknown", status, false),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub platform: String,
    pub last_seen_at: String,
}

// .trellis/.runtime/sessions/*.json 的原始结构
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawSession {
    platform: String,
    last_seen_at: String,
    current_task: Option<String>,
}

// 读取项目的全部会话文件，返回 任务目录名 → 会话列表
fn scan_sessions(project_dir: &Path) -> std::collections::HashMap<String, Vec<SessionInfo>> {
    let mut map: std::collections::HashMap<String, Vec<SessionInfo>> = Default::default();
    let dir = project_dir
        .join(".trellis")
        .join(".runtime")
        .join("sessions");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return map,
    };
    for e in entries.flatten() {
        if e.path().extension().map(|x| x == "json").unwrap_or(false) {
            let raw: RawSession = fs::read_to_string(e.path())
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            // current_task 形如 ".trellis/tasks/<dir>"（Windows 可能是反斜杠，先归一化）
            let task_dir = raw
                .current_task
                .as_deref()
                .map(crate::platform::to_posix)
                .and_then(|p| p.rsplit('/').next().map(str::to_owned))
                .unwrap_or_default();
            if !task_dir.is_empty() && !raw.last_seen_at.is_empty() {
                map.entry(task_dir.to_string())
                    .or_default()
                    .push(SessionInfo {
                        platform: raw.platform,
                        last_seen_at: raw.last_seen_at,
                    });
            }
        }
    }
    map
}

// 从 prd.md 提取摘要：跳过标题/空行/引用标记，取第一段正文，截到 max_len
// 去掉行首的无序/有序列表标记（-、*、+、1. 等），返回 (内容, 是否列表项)
fn strip_bullet(s: &str) -> (&str, bool) {
    let b = s.trim_start_matches(['-', '*', '+']).trim_start();
    if b.len() != s.len() {
        return if b.is_empty() { (s, false) } else { (b, true) };
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && bytes.get(i) == Some(&b'.') && bytes.get(i + 1) == Some(&b' ') {
        return (s[i + 2..].trim_start(), true);
    }
    (s, false)
}

fn prd_excerpt(prd_path: &Path, max_len: usize) -> String {
    let content = match fs::read_to_string(prd_path) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let mut out = String::new();
    let mut started = false;
    let mut prev_bullet = false;
    for line in content.lines() {
        let t = line.trim();
        // 空行=段落结束：已开始累积就停，未开始继续找
        if t.is_empty() {
            if started {
                break;
            }
            continue;
        }
        if t.starts_with('#') || t.starts_with('>') || t.starts_with("---") {
            if started {
                break;
            }
            continue;
        }
        // 跳过 PRD 头部的元信息行（Parent: `xxx` / 状态： 之类），不含信息量
        let lower = t.trim_start_matches('-').trim().to_lowercase();
        if [
            "parent",
            "project",
            "created",
            "updated",
            "status",
            "branch",
            "id",
            "状态",
            "进度",
            "父任务",
            "项目",
            "分支",
        ]
        .iter()
        .any(|k| lower.starts_with(&format!("{}:", k)) || lower.starts_with(&format!("{}：", k)))
        {
            if started {
                break;
            }
            continue;
        }
        let (body, is_bullet) = strip_bullet(t);
        // 列表项之间用「；」分隔；英文硬换行补空格；中文直接拼接
        if started {
            if prev_bullet || is_bullet {
                out.push('；');
            } else if out
                .chars()
                .last()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or(false)
                && body
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_alphanumeric())
                    .unwrap_or(false)
            {
                out.push(' ');
            }
        }
        out.push_str(body);
        started = true;
        prev_bullet = is_bullet;
        if out.chars().count() > max_len {
            break;
        }
    }
    if out.chars().count() > max_len {
        let cut: String = out.chars().take(max_len).collect();
        format!("{}…", cut.trim_end())
    } else {
        out
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub name: String,
    pub path: String,
    pub task_count: usize,
    pub last_activity: Option<String>,
}

fn is_directory(p: &Path) -> bool {
    fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

/* 判断是否为 Trellis 初始化自带的 Bootstrap Guidelines 引导任务目录
（默认目录名 00-bootstrap-guidelines）；对任务观察无意义，扫描时过滤。 */
fn is_bootstrap_guidelines(dir_name: &str) -> bool {
    dir_name == "00-bootstrap-guidelines" || dir_name.ends_with("bootstrap-guidelines")
}

// 在 root 下最多 max_depth 层寻找含 .trellis/tasks/ 的目录（BFS，root 自身为 depth 0）
// 找到项目后仍继续深入子目录，以便发现嵌套的 Trellis 项目（如 monorepo 中
// 子包自带 .trellis）；max_depth 兜底限制扫描深度。
pub fn discover_projects(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if !is_directory(root) {
        return found;
    }
    let root = root.to_path_buf();
    let mut queue = VecDeque::from([(root, 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        if depth > max_depth {
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if is_directory(&dir.join(".trellis").join("tasks")) {
            found.push(dir);
        }
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if !path.is_dir() {
                continue;
            }
            if SKIP_DIRS.contains(&name.as_str()) || name.starts_with('.') {
                continue;
            }
            queue.push_back((path, depth + 1));
        }
    }
    found
}

fn mtime_ms(p: &Path) -> i64 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

struct TaskLocation<'a> {
    rel_dir: &'a str,
    archived: bool,
}

// 归一化 + 计算字段（progress/stage/lane/kind/sessions/excerpt/artifacts/phase）
// rel_dir：任务目录相对 .trellis/tasks/ 的路径（归档任务形如 archive/2026-08/<id>，活跃任务为 <id>）
// archived：任务是否位于 archive/ 下
fn build_task(
    dir_name: &str,
    raw: RawTask,
    mtime: i64,
    sessions: Vec<SessionInfo>,
    excerpt: String,
    artifacts: Artifacts,
    location: TaskLocation<'_>,
) -> Task {
    let status = raw.status.clone().unwrap_or_else(|| "planning".into());
    let progress = crate::progress::compute_progress(&status, &raw.subtasks);
    let (lane, kind) = crate::progress::lane_model(&status);
    let phase = infer_phase(&status, &artifacts);
    Task {
        id: raw.id.clone().unwrap_or_else(|| dir_name.to_string()),
        title: raw.title.clone().unwrap_or_else(|| dir_name.to_string()),
        description: raw.description.unwrap_or_default(),
        status: status.clone(),
        priority: raw.priority.unwrap_or_else(|| "P2".into()),
        dev_type: raw.dev_type,
        scope: raw.scope,
        package: raw.package,
        branch: raw.branch,
        parent: raw.parent,
        children: raw.children,
        subtasks: raw.subtasks,
        created_at: raw.created_at,
        completed_at: raw.completed_at,
        mtime,
        progress,
        stage: crate::progress::growth_stage(&status).to_string(),
        lane,
        partial: progress,
        kind: kind.to_string(),
        archived: location.archived,
        sessions,
        excerpt,
        artifacts,
        phase,
        dir: location.rel_dir.to_string(),
    }
}

// 解析单个项目的全部活跃任务；损坏/缺文件的任务跳过并记入 errors；隐藏目录不扫。
// 与 scan_tasks_with_archived 的区别：不扫描 archive/ 目录（保持 task_count / version 语义不变）。
pub fn scan_tasks(project_dir: &Path) -> (Vec<Task>, Vec<String>) {
    scan_tasks_inner(project_dir, false)
}

// 解析单个项目的全部任务，含 archive/ 下的已归档任务（archived=true，仅查看，不可反归档）。
// 前端勾选「显示已归档」时用此结果。
pub fn scan_tasks_with_archived(project_dir: &Path) -> (Vec<Task>, Vec<String>) {
    scan_tasks_inner(project_dir, true)
}

// 只扫描 archive/ 下的已归档任务（懒加载：前端勾选「显示已归档」或聚焦归档任务时才调用，
// 避免常规轮询每次都扫 archive/ 拖慢全项目扫描）。version 语义由调用方单独计算。
pub fn scan_archived(project_dir: &Path) -> (Vec<Task>, Vec<String>) {
    let tasks_dir = project_dir.join(".trellis").join("tasks");
    let mut tasks = Vec::new();
    let mut errors = Vec::new();
    let archive_dir = tasks_dir.join("archive");
    if is_directory(&archive_dir) {
        let sessions = scan_sessions(project_dir);
        scan_archived_dir(&tasks_dir, &archive_dir, &sessions, &mut tasks, &mut errors);
    }
    tasks.sort_by(|a, b| {
        a.created_at
            .clone()
            .unwrap_or_default()
            .cmp(&b.created_at.clone().unwrap_or_default())
            .then_with(|| a.id.cmp(&b.id))
    });
    (tasks, errors)
}

fn scan_tasks_inner(project_dir: &Path, include_archived: bool) -> (Vec<Task>, Vec<String>) {
    let tasks_dir = project_dir.join(".trellis").join("tasks");
    let mut tasks = Vec::new();
    let mut errors = Vec::new();
    if !is_directory(&tasks_dir) {
        return (tasks, errors);
    }
    let sessions = scan_sessions(project_dir);
    let entries = match fs::read_dir(&tasks_dir) {
        Ok(e) => e,
        Err(err) => {
            errors.push(format!("无法读取 {}: {}", tasks_dir.display(), err));
            return (tasks, errors);
        }
    };
    for e in entries.flatten() {
        let dir_name = e.file_name().to_string_lossy().into_owned();
        let task_path = e.path();
        /* archive/ 目录：默认跳过；include_archived 时递归扫描其中的任务 */
        if dir_name == "archive" {
            if include_archived {
                scan_archived_dir(&tasks_dir, &task_path, &sessions, &mut tasks, &mut errors);
            }
            continue;
        }
        if !task_path.is_dir() || dir_name.starts_with('.') {
            continue;
        }
        /* 过滤 Trellis 初始化自带的 Bootstrap Guidelines 引导任务：目录名以
        bootstrap-guidelines 结尾（00-bootstrap-guidelines），对观察无意义。 */
        if is_bootstrap_guidelines(&dir_name) {
            continue;
        }
        scan_task_dir(
            &dir_name,
            &dir_name,
            false,
            &task_path,
            &sessions,
            &mut tasks,
            &mut errors,
        );
    }
    tasks.sort_by(|a, b| {
        a.created_at
            .clone()
            .unwrap_or_default()
            .cmp(&b.created_at.clone().unwrap_or_default())
            .then_with(|| a.id.cmp(&b.id))
    });
    (tasks, errors)
}

// 递归扫描 archive/ 下的任务：archive/ 下一层是年月（如 2026-08），再一层才是任务目录。
fn scan_archived_dir(
    tasks_dir: &Path,
    archive_dir: &Path,
    sessions: &std::collections::HashMap<String, Vec<SessionInfo>>,
    tasks: &mut Vec<Task>,
    errors: &mut Vec<String>,
) {
    let entries = match fs::read_dir(archive_dir) {
        Ok(e) => e,
        Err(err) => {
            errors.push(format!("无法读取 {}: {}", archive_dir.display(), err));
            return;
        }
    };
    for e in entries.flatten() {
        let dir_name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if !path.is_dir() || dir_name.starts_with('.') {
            continue;
        }
        /* 年月子目录（archive/2026-08/）：继续深入一层找任务 */
        if !path.join("task.json").is_file() {
            scan_archived_dir(tasks_dir, &path, sessions, tasks, errors);
            continue;
        }
        if is_bootstrap_guidelines(&dir_name) {
            continue;
        }
        /* 任务目录：rel_dir 保留相对 tasks/ 的完整路径（含年月段），供 get_task 定位文档 */
        let full_rel = relative_to(tasks_dir, &path);
        scan_task_dir(&dir_name, &full_rel, true, &path, sessions, tasks, errors);
    }
}

// 计算 path 相对 tasks_dir 的路径（如 archive/2026-08/<id>）
fn relative_to(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "archive".into())
}

// 解析单个任务目录并加入 tasks；rel_dir 相对 .trellis/tasks/，archived 标记归档归属。
fn scan_task_dir(
    dir_name: &str,
    rel_dir: &str,
    archived: bool,
    task_path: &Path,
    sessions: &std::collections::HashMap<String, Vec<SessionInfo>>,
    tasks: &mut Vec<Task>,
    errors: &mut Vec<String>,
) {
    let task_json_path = task_path.join("task.json");
    let raw: RawTask = match fs::read_to_string(&task_json_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(r) => r,
        None => {
            errors.push(format!("跳过任务 {}", dir_name));
            return;
        }
    };
    // 归档任务只读，不继承同 id 活跃任务的会话状态。
    let task_sessions = if archived {
        Vec::new()
    } else {
        sessions.get(dir_name).cloned().unwrap_or_default()
    };
    let excerpt = prd_excerpt(&task_path.join("prd.md"), 400);
    let artifacts = scan_artifacts(task_path);
    tasks.push(build_task(
        dir_name,
        raw,
        mtime_ms(&task_json_path),
        task_sessions,
        excerpt,
        artifacts,
        TaskLocation { rel_dir, archived },
    ));
}

pub fn project_info(project_dir: &Path) -> ProjectInfo {
    let (tasks, _) = scan_tasks(project_dir);
    let last = tasks.iter().map(|t| t.mtime).max().unwrap_or(0);
    ProjectInfo {
        name: project_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        /* Windows canonicalize 会带 \\?\ 设备路径前缀，去掉它再展示 */
        path: crate::platform::strip_device_prefix(&project_dir.to_string_lossy()),
        task_count: tasks.len(),
        last_activity: if last > 0 {
            chrono::DateTime::from_timestamp_millis(last).map(|t| t.to_rfc3339())
        } else {
            None
        },
    }
}

// list_tasks 的 version：最大 mtime
pub fn tasks_version(tasks: &[Task]) -> String {
    tasks.iter().map(|t| t.mtime).max().unwrap_or(0).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Fixture {
        root: PathBuf,
        alpha: PathBuf,
    }

    fn setup(tag: &str) -> Fixture {
        let root =
            std::env::temp_dir().join(format!("trellis-card-scan-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&root);

        // 深度 2 的合法项目：root/work/alpha
        let alpha = root.join("work").join("alpha");
        let t1 = alpha.join(".trellis/tasks/01-a");
        fs::create_dir_all(&t1).unwrap();
        fs::write(
            t1.join("task.json"),
            r#"{
              "id": "01-a", "title": "Alpha task", "status": "in_progress",
              "priority": "P1", "dev_type": "backend", "createdAt": "2026-06-01",
              "subtasks": [{"name":"s1","status":"completed"},{"name":"s2","status":"pending"}]
            }"#,
        )
        .unwrap();
        // 损坏的 task.json
        let t2 = alpha.join(".trellis/tasks/02-broken");
        fs::create_dir_all(&t2).unwrap();
        fs::write(t2.join("task.json"), "{broken").unwrap();
        // 缺 task.json 的目录
        fs::create_dir_all(alpha.join(".trellis/tasks/03-nojson")).unwrap();
        // Bootstrap Guidelines 引导任务：有合法 task.json，但应被过滤
        let tb = alpha.join(".trellis/tasks/00-bootstrap-guidelines");
        fs::create_dir_all(&tb).unwrap();
        fs::write(
            tb.join("task.json"),
            r#"{"id":"00-bootstrap-guidelines","title":"Bootstrap Guidelines","status":"completed"}"#,
        )
        .unwrap();
        // 隐藏目录（如 .omc）应静默跳过，不计入 errors
        fs::create_dir_all(alpha.join(".trellis/tasks/.omc")).unwrap();
        // archive 目录不应被扫
        let ta = alpha.join(".trellis/tasks/archive/2026-02/old");
        fs::create_dir_all(&ta).unwrap();
        fs::write(ta.join("task.json"), r#"{"id":"old","status":"completed"}"#).unwrap();
        let archived_bootstrap =
            alpha.join(".trellis/tasks/archive/2026-02/00-bootstrap-guidelines");
        fs::create_dir_all(&archived_bootstrap).unwrap();
        fs::write(
            archived_bootstrap.join("task.json"),
            r#"{"id":"00-bootstrap-guidelines","title":"Bootstrap Guidelines","status":"completed"}"#,
        )
        .unwrap();

        // 不含 .trellis 的普通目录
        fs::create_dir_all(root.join("work/beta")).unwrap();
        // 超过深度 3 的项目
        fs::create_dir_all(root.join("a/b/c/d/deep-proj/.trellis/tasks")).unwrap();
        // node_modules 里的假项目
        fs::create_dir_all(root.join("work/node_modules/fake/.trellis/tasks")).unwrap();

        Fixture { root, alpha }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn discover_finds_projects_within_depth() {
        let f = setup("discover");
        let found = discover_projects(&f.root, 3);
        assert_eq!(found, vec![f.alpha.clone()]);
    }

    #[test]
    fn discover_missing_root_returns_empty() {
        let f = setup("missing");
        assert!(discover_projects(&f.root.join("nope"), 3).is_empty());
    }

    #[test]
    fn discover_finds_nested_trellis_projects_inside_project() {
        /* 上级是 Trellis 项目，内部子目录还有自己的 .trellis —— 两者都应被发现 */
        let root = std::env::temp_dir().join(format!(
            "trellis-card-nested-discover-{}",
            std::process::id()
        ));
        let parent = root.join("repo");
        let child = parent.join("packages").join("sub");
        fs::create_dir_all(parent.join(".trellis/tasks")).unwrap();
        fs::create_dir_all(child.join(".trellis/tasks")).unwrap();

        let found = discover_projects(&root, 4);
        assert!(found.contains(&parent));
        assert!(found.contains(&child));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scan_parses_valid_and_skips_broken() {
        let f = setup("scan");
        let (tasks, errors) = scan_tasks(&f.alpha);
        assert_eq!(tasks.len(), 1);
        assert_eq!(errors.len(), 2); // broken + nojson；archive 静默跳过
        let t = &tasks[0];
        assert_eq!(t.id, "01-a");
        assert_eq!(t.title, "Alpha task");
        assert_eq!(t.dev_type.as_deref(), Some("backend"));
        assert_eq!(t.subtasks.len(), 2);
        assert!(t.mtime > 0);
        assert_eq!(t.progress, 0.5);
        assert_eq!(t.kind, "work");
    }

    #[test]
    fn scan_with_archived_includes_archive_tasks_marked() {
        let f = setup("scan-archived");
        /* archive/2026-02/old 的 task.json: {"id":"old","status":"completed"} */
        let (tasks, errors) = scan_tasks_with_archived(&f.alpha);
        assert_eq!(errors.len(), 2, "应只报 fixture 中两个损坏的活跃任务");
        /* 活跃任务仍在 */
        assert!(tasks.iter().any(|t| t.id == "01-a" && !t.archived));
        /* 归档任务已包含，且 marked archived=true */
        let archived: Vec<_> = tasks.iter().filter(|t| t.archived).collect();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, "old");
        /* 归档任务 dir 保留相对 tasks/ 的完整路径，get_task 才能定位文档 */
        let old = archived.iter().find(|t| t.id == "old").unwrap();
        assert_eq!(old.dir, "archive/2026-02/old");
        /* 归档任务 status 为 completed，不计入 active */
        assert_eq!(old.status, "completed");
    }

    #[test]
    fn scan_dir_without_trellis_returns_empty() {
        let f = setup("notrellis");
        let (tasks, errors) = scan_tasks(&f.root.join("work/beta"));
        assert!(tasks.is_empty() && errors.is_empty());
    }

    #[test]
    fn scan_archived_returns_only_archive_tasks() {
        let f = setup("scan-archived-only");
        /* 懒加载路径：scan_archived 只扫 archive/，不返回任何活跃任务 */
        let (tasks, errors) = scan_archived(&f.alpha);
        assert!(errors.is_empty(), "懒加载不应复现活跃任务的损坏错误");
        let archived: Vec<_> = tasks.iter().filter(|t| t.archived).collect();
        /* fixture 中 archive/2026-02/old 有效；00-bootstrap-guidelines 被过滤 */
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, "old");
        /* 活跃任务 01-a 不在结果里 */
        assert!(!tasks.iter().any(|t| t.id == "01-a"));
        /* dir 保留相对 tasks/ 完整路径 */
        assert_eq!(archived[0].dir, "archive/2026-02/old");
    }

    #[test]
    fn scan_filters_bootstrap_guidelines_task() {
        let f = setup("bootstrap");
        /* fixture 里 00-bootstrap-guidelines 有合法 task.json，但应被过滤 */
        let (tasks, _) = scan_tasks(&f.alpha);
        assert!(
            !tasks.iter().any(|t| t.id == "00-bootstrap-guidelines"),
            "Bootstrap Guidelines 任务不应出现在扫描结果中"
        );
        assert!(tasks.iter().any(|t| t.id == "01-a"));
    }

    #[test]
    fn is_bootstrap_guidelines_matches_known_dir_names() {
        assert!(is_bootstrap_guidelines("00-bootstrap-guidelines"));
        assert!(is_bootstrap_guidelines("42-bootstrap-guidelines"));
        assert!(!is_bootstrap_guidelines("02-27-user-login"));
        assert!(!is_bootstrap_guidelines("bootstrap"));
    }

    #[test]
    fn normalize_defaults_for_legacy_task() {
        let f = setup("legacy");
        let dir = f.alpha.join(".trellis/tasks/09-x");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("task.json"), r#"{"title":"Only title"}"#).unwrap();
        let (tasks, _) = scan_tasks(&f.alpha);
        let t = tasks.iter().find(|t| t.id == "09-x").unwrap();
        assert_eq!(t.status, "planning");
        assert_eq!(t.priority, "P2");
        assert_eq!(t.dev_type, None);
        assert!(t.subtasks.is_empty());
        assert_eq!(t.branch, None);
        assert_eq!(t.lane, 0);
        assert_eq!(t.kind, "plan");
    }

    #[test]
    fn project_info_reports_name_count_activity() {
        let f = setup("pinfo");
        let info = project_info(&f.alpha);
        assert_eq!(info.name, "alpha");
        assert_eq!(info.task_count, 1);
        assert!(info.last_activity.is_some());
    }

    #[test]
    fn scan_attaches_sessions_and_prd_excerpt() {
        let root = std::env::temp_dir().join(format!("trellis-card-sess-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let proj = root.join("proj");
        let task_dir = proj.join(".trellis/tasks/01-x");
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            task_dir.join("task.json"),
            r#"{"id":"01-x","status":"in_progress"}"#,
        )
        .unwrap();
        fs::write(
            task_dir.join("prd.md"),
            "# 标题\n\n## Goal\n\n做一个名次区间的功能，支持第 m 到第 n 名完成。\n\n## 其他\n",
        )
        .unwrap();
        let sess_dir = proj.join(".trellis/.runtime/sessions");
        fs::create_dir_all(&sess_dir).unwrap();
        fs::write(
            sess_dir.join("claude_abc.json"),
            r#"{"platform":"claude","last_seen_at":"2026-07-20T09:12:17Z","current_task":".trellis/tasks/01-x","current_run":null}"#,
        )
        .unwrap();
        // 指向不存在任务的会话应被忽略
        fs::write(
            sess_dir.join("codex_def.json"),
            r#"{"platform":"codex","last_seen_at":"2026-07-15T10:13:03Z","current_task":".trellis/tasks/99-gone","current_run":null}"#,
        )
        .unwrap();

        let (tasks, _) = scan_tasks(&proj);
        assert_eq!(tasks.len(), 1);
        let t = &tasks[0];
        assert_eq!(t.sessions.len(), 1);
        assert_eq!(t.sessions[0].platform, "claude");
        assert_eq!(t.sessions[0].last_seen_at, "2026-07-20T09:12:17Z");
        assert!(
            t.excerpt.starts_with("做一个名次区间的功能"),
            "excerpt = {}",
            t.excerpt
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prd_excerpt_skips_headings_and_truncates() {
        let p = std::env::temp_dir().join(format!("trellis-card-prd-{}.md", std::process::id()));
        fs::write(&p, "# T\n\n> 引用也跳过\n\n正文第一段。\n").unwrap();
        assert_eq!(prd_excerpt(&p, 160), "正文第一段。");
        let long = format!("# T\n\n{}\n", "很长".repeat(200));
        fs::write(&p, long).unwrap();
        let ex = prd_excerpt(&p, 50);
        assert!(ex.ends_with('…'));
        assert!(ex.chars().count() <= 52);
        assert_eq!(prd_excerpt(Path::new("/nonexistent"), 10), "");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn prd_excerpt_skips_meta_lines() {
        let p =
            std::env::temp_dir().join(format!("trellis-card-prd-meta-{}.md", std::process::id()));
        fs::write(
            &p,
            "# T\n\nParent: `07-23-interactive-session`\n\n- Status: in_progress\n\n真正的内容。\n",
        )
        .unwrap();
        assert_eq!(prd_excerpt(&p, 160), "真正的内容。");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn prd_excerpt_joins_paragraph_lines() {
        let p =
            std::env::temp_dir().join(format!("trellis-card-prd-para-{}.md", std::process::id()));
        // 中文硬换行直接拼；英文硬换行补空格；段落止于空行
        fs::write(&p, "# T\n\n第一行内容，\n第二行内容。\n\n第二段不要。\n").unwrap();
        assert_eq!(prd_excerpt(&p, 400), "第一行内容，第二行内容。");
        fs::write(&p, "# T\n\nwraps onto\nthe next line\n").unwrap();
        assert_eq!(prd_excerpt(&p, 400), "wraps onto the next line");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn prd_excerpt_strips_bullets_and_chinese_meta() {
        let p =
            std::env::temp_dir().join(format!("trellis-card-prd-bul-{}.md", std::process::id()));
        // 中文元信息行跳过；列表项去标记后用「；」连接
        fs::write(
            &p,
            "# T\n\n- 状态： 已完成\n\n- 已完成：梳理结构\n- 验证通过\n",
        )
        .unwrap();
        assert_eq!(prd_excerpt(&p, 400), "已完成：梳理结构；验证通过");
        fs::write(&p, "# T\n\n1. 第一步\n2. 第二步\n").unwrap();
        assert_eq!(prd_excerpt(&p, 400), "第一步；第二步");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn jsonl_entries_skip_seed_lines() {
        let p =
            std::env::temp_dir().join(format!("trellis-card-jsonl-{}.jsonl", std::process::id()));
        fs::write(
            &p,
            "{\"_example\": \"Fill with file/reason...\"}\n{\"file\": \".trellis/spec/a.md\", \"reason\": \"x\"}\n\n{\"file\": \"research/b.md\"}\n{bad json\n",
        )
        .unwrap();
        assert_eq!(count_jsonl_entries(&p), 2);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn artifacts_and_phase_inference() {
        let root = std::env::temp_dir().join(format!("trellis-card-art-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let proj = root.join("proj");

        // 任务 1：刚建，只有空 task.json → 需求探索中
        let t1 = proj.join(".trellis/tasks/01-new");
        fs::create_dir_all(&t1).unwrap();
        fs::write(
            t1.join("task.json"),
            r#"{"id":"01-new","status":"planning"}"#,
        )
        .unwrap();

        // 任务 2：有 prd 但 jsonl 只有 seed → 规范注入未配 ⚠
        let t2 = proj.join(".trellis/tasks/02-warn");
        fs::create_dir_all(&t2).unwrap();
        fs::write(
            t2.join("task.json"),
            r#"{"id":"02-warn","status":"planning"}"#,
        )
        .unwrap();
        fs::write(t2.join("prd.md"), "# PRD\n\n内容\n").unwrap();
        fs::write(t2.join("implement.jsonl"), "{\"_example\": \"...\"}\n").unwrap();

        // 任务 3：prd + 真实 jsonl 条目 → 规划就绪
        let t3 = proj.join(".trellis/tasks/03-ready");
        fs::create_dir_all(&t3).unwrap();
        fs::write(
            t3.join("task.json"),
            r#"{"id":"03-ready","status":"planning"}"#,
        )
        .unwrap();
        fs::write(t3.join("prd.md"), "# PRD\n\n内容\n").unwrap();
        fs::write(
            t3.join("implement.jsonl"),
            "{\"_example\": \"...\"}\n{\"file\": \".trellis/spec/x.md\", \"reason\": \"y\"}\n",
        )
        .unwrap();
        fs::create_dir_all(t3.join("research")).unwrap();
        fs::write(t3.join("research/lib.md"), "# 调研\n").unwrap();

        // 任务 4：in_progress + 验收报告 → 验证阶段
        let t4 = proj.join(".trellis/tasks/04-verify");
        fs::create_dir_all(&t4).unwrap();
        fs::write(
            t4.join("task.json"),
            r#"{"id":"04-verify","status":"in_progress"}"#,
        )
        .unwrap();
        fs::write(t4.join("acceptance-report-final.md"), "# 报告\n").unwrap();

        let (tasks, _) = scan_tasks(&proj);
        let get = |id: &str| tasks.iter().find(|t| t.id == id).unwrap();

        assert_eq!(get("01-new").phase.id, "explore");
        assert!(!get("01-new").phase.warn);

        let w = get("02-warn");
        assert_eq!(w.phase.id, "need-context");
        assert!(w.phase.warn);
        assert!(w.artifacts.prd);
        assert_eq!(w.artifacts.impl_entries, 0);

        let r = get("03-ready");
        assert_eq!(r.phase.id, "ready");
        assert_eq!(r.artifacts.impl_entries, 1);
        assert_eq!(r.artifacts.research_count, 1);

        let v = get("04-verify");
        assert_eq!(v.phase.id, "verify");
        assert_eq!(v.artifacts.report_count, 1);

        // 任务 5：有 PRD/design/research 但没有真实 implement 条目，仍未完成规范注入
        let t5 = proj.join(".trellis/tasks/05-docs-only");
        fs::create_dir_all(t5.join("research")).unwrap();
        fs::write(
            t5.join("task.json"),
            r#"{"id":"05-docs-only","status":"planning"}"#,
        )
        .unwrap();
        fs::write(t5.join("prd.md"), "# PRD\n\n内容\n").unwrap();
        fs::write(t5.join("design.md"), "# Design\n").unwrap();
        fs::write(t5.join("research/source.md"), "# Research\n").unwrap();
        let (tasks_after, _) = scan_tasks(&proj);
        let docs_only = tasks_after.iter().find(|t| t.id == "05-docs-only").unwrap();
        assert_eq!(docs_only.phase.id, "need-context");
        assert!(docs_only.phase.warn);

        let _ = fs::remove_dir_all(&root);
    }
}
