#!/usr/bin/env python3
"""
验证规范预览改动：三列网格 + rel-spec-body 复用 .doc markdown 排版。
连接 tauri dev 本地前端，注入 mock 数据（含丰富 markdown 的 specs），
切到「规范」tab 截图；验证 3 列布局与窄窗口压缩。
"""
import json
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

DEV_URL = "http://127.0.0.1:1430/"
OUT_DIR = Path(__file__).resolve().parent.parent / "docs" / "screenshots" / "spec-preview-grid"
OUT_DIR.mkdir(parents=True, exist_ok=True)


def wait_for_dev_server(url=DEV_URL, timeout=60):
    start = time.time()
    while time.time() - start < timeout:
        try:
            with urllib.request.urlopen(url, timeout=2) as resp:
                if resp.status == 200:
                    return
        except Exception:
            time.sleep(0.5)
    raise RuntimeError(f"dev server {url} not ready in {timeout}s")


def build_mocks():
    now = int(time.time())
    project_name = "demo"
    project_path = "/tmp/demo"
    task_id = "01-auth"
    task_key = f"{project_name}::{task_id}"

    project = {
        "name": project_name,
        "path": project_path,
        "taskCount": 3,
        "lastActivity": "2026-08-11 20:00:00",
    }

    task = {
        "id": task_id,
        "title": "用户登录认证",
        "description": "实现基于 JWT 的用户登录、注册与 token 刷新机制，包含前端表单与后端接口联调。",
        "status": "in_progress",
        "priority": "P1",
        "devType": None,
        "scope": None,
        "package": None,
        "branch": "feature/auth",
        "parent": None,
        "children": [],
        "subtasks": [],
        "createdAt": "2026-08-01 10:00:00",
        "completedAt": None,
        "mtime": now,
        "progress": 0.55,
        "stage": "work",
        "lane": 1,
        "partial": 0.55,
        "kind": "work",
        "archived": False,
        "sessions": [
            {"platform": "claude", "lastSeenAt": "2026-08-11T20:00:00Z"}
        ],
        "excerpt": "登录模块需要支持邮箱+密码、OAuth GitHub 两种方式，token 有效期 24 小时。",
        "artifacts": {
            "prd": True, "design": True, "implement": True,
            "researchCount": 2, "implEntries": 12, "checkEntries": 3, "reportCount": 0,
        },
        "specRefs": [],
        "fileRefs": [],
        "prdRefs": [],
        "phase": {"id": "implement", "label": "实现", "warn": False},
        "dir": task_id,
    }

    runtime_view = {
        "project": project_name,
        "taskId": task_id,
        "taskStatus": "in_progress",
        "phase": "implement",
        "displayState": "working",
        "attention": "normal",
        "confidence": "high",
        "action": "implement",
        "agent": {"sessionId": "sess-claude-001", "agentKind": "claude", "project": project_name,
                  "taskId": task_id, "eventName": "postToolUse", "state": "working", "waitingReason": None,
                  "toolName": "Bash", "toolInput": {"command": "cargo test", "filePath": None},
                  "activity": "运行测试中…", "startedAt": now - 300, "updatedAt": now},
        "activity": "运行测试中…",
        "focusScore": 95,
        "lastChangedAt": now,
    }

    snapshot = {"tasks": [runtime_view], "projectActivities": [], "errors": [],
                "focusKey": task_key, "generatedAt": now}

    hooks = [
        {"agent": "codex", "installed": False, "configPath": "~/.codex/hooks.json"},
        {"agent": "claude", "installed": True, "configPath": "~/.claude/settings.json"},
        {"agent": "cursor", "installed": False, "configPath": "~/.cursor/hooks.json"},
        {"agent": "pi", "installed": False, "configPath": "~/.pi/agent/extensions/trellis-card.ts"},
        {"agent": "opencode", "installed": False, "configPath": "~/.config/opencode/plugins/trellis-card.js"},
    ]

    rich_md = (
        "# 任务规范\n\n"
        "## 目标\n\n"
        "这是**粗体**与`行内代码`的示例，以及 [链接](https://example.com)。\n\n"
        "## 关键决策\n\n"
        "- 使用 JWT 做认证\n"
        "- token 有效期 **24 小时**\n"
        "- 支持邮箱 + OAuth GitHub\n\n"
        "## 代码示例\n\n"
        "```js\n"
        "const token = sign({ uid }, SECRET, { expiresIn: '24h' });\n"
        "console.log(token);\n"
        "```\n\n"
        "## 注意事项\n\n"
        "> 不要在日志中打印 token。\n\n"
        "| 字段 | 类型 | 说明 |\n"
        "| --- | --- | --- |\n"
        "| uid | string | 用户 ID |\n"
        "| role | enum | 角色 |\n"
        "| ts | number | 时间戳 |\n\n"
        "- [x] 登录接口\n"
        "- [ ] 回归测试\n"
    )

    # 5 个已沉淀（filled）+ 2 个空模板
    specs = [
        {"name": "任务管理规范.md", "path": "tasks/管理规范.md", "category": "tasks", "filled": True,
         "lineCount": 42, "content": rich_md},
        {"name": "API 接口规范.md", "path": "api/接口规范.md", "category": "api", "filled": True,
         "lineCount": 120, "content": "## 认证\n\n使用 Bearer token，见 [文档](https://example.com)。\n\n## 限流\n\n```python\nRATE = 100\n```"},
        {"name": "代码风格.md", "path": "code/风格.md", "category": "code", "filled": True,
         "lineCount": 66, "content": "## 命名\n\n- 变量：`camelCase`\n- 常量：`UPPER_SNAKE`\n\n## 缩进\n\n使用 **2 空格**。"},
        {"name": "数据库迁移.md", "path": "db/迁移.md", "category": "db", "filled": True,
         "lineCount": 30, "content": "## 版本管理\n\n> 迁移必须带时间戳前缀。\n\n`2026_08_01_init.sql`"},
        {"name": "部署流程.md", "path": "deploy/流程.md", "category": "deploy", "filled": True,
         "lineCount": 55, "content": "## 发布\n\n1. 构建\n2. 测试\n3. 灰度\n\n## 回滚\n\n`git revert` 后强制走 CI。"},
        {"name": "测试规范.md", "path": "test/规范.md", "category": "test", "filled": False,
         "lineCount": 0, "content": ""},
        {"name": "日志规范.md", "path": "log/规范.md", "category": "log", "filled": False,
         "lineCount": 0, "content": ""},
    ]

    handlers = {
        "complete_setup": {},
        "list_projects": [project],
        "list_tasks": {"version": "v1", "tasks": [task], "errors": []},
        "list_relations": {"tasks": [], "specGroups": []},
        "list_specs": specs,
        "list_archived": {"version": "", "tasks": []},
        "get_runtime_snapshot": snapshot,
        "get_version": "0.1.6",
        "get_task": {**task, "docs": []},
        "archive_task": {},
        "get_hook_statuses": hooks,
        "configure_hook": {},
        "set_always_on_top": {},
        "set_window_mode": {},
        "set_capsule_expanded": {},
        "set_window_size": {},
        "fit_window_height": {},
        "hide_window": {},
        "open_url": {},
    }
    return handlers


def tauri_mock_script(handlers_json: str) -> str:
    return f"""
    window.__TAURI__ = window.__TAURI__ || {{}};
    window.__TAURI__.core = window.__TAURI__.core || {{}};
    const __TAURI_MOCKS__ = {handlers_json};
    window.__TAURI__.core.invoke = function(cmd, args) {{
      const data = __TAURI_MOCKS__[cmd];
      const payload = data === undefined ? null : (typeof data === 'function' ? data(args) : data);
      return Promise.resolve(payload);
    }};
    window.__TAURI__.event = window.__TAURI__.event || {{}};
    window.__TAURI__.event.listen = function() {{ return Promise.resolve(function() {{}}); }};
    """


def screenshot(name, page):
    path = OUT_DIR / f"{name}.png"
    page.screenshot(path=path, full_page=False)
    print(f"saved: {path}")
    return path


def main():
    wait_for_dev_server()
    handlers = build_mocks()
    init_script = tauri_mock_script(json.dumps(handlers, ensure_ascii=False))

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        context = browser.new_context(viewport={"width": 940, "height": 900})
        page = context.new_page()
        page.add_init_script(init_script)
        page.goto(DEV_URL)

        page.wait_for_selector("#btnStartEmpty", timeout=10000)
        page.click("#btnStartEmpty")
        page.wait_for_selector("#mainView", state="visible", timeout=10000)
        page.wait_for_timeout(800)

        # 关掉 admin / scrim
        admin_close = page.locator("#admin .list-h button[title='关闭'], #admin .list-h .iconbtn").first
        if admin_close.is_visible():
            admin_close.click()
            page.wait_for_timeout(300)
        scrim = page.locator("#menuScrim")
        if scrim.is_visible():
            scrim.click()
            page.wait_for_timeout(200)

        # 打开关联视图 sheet（rel-canvas 940 宽），等 sheet 稳定
        rel_btn = page.locator("#btnRelations")
        if rel_btn.is_visible():
            rel_btn.click()
            page.wait_for_selector("#relations.open", timeout=5000)
            page.wait_for_timeout(800)

        # 切到「规范」tab，等 grid 出现
        page.click(".view-tab[data-view='specs']")
        page.wait_for_selector(".rel-spec-grid", timeout=5000)
        page.wait_for_timeout(800)
        screenshot("01-specs-3col", page)

        # 宽窗口下再次截图（验证不同宽度列数一致）
        page.set_viewport_size({"width": 1600, "height": 900})
        page.wait_for_timeout(500)
        screenshot("02-specs-wide", page)

        # 窄窗口：验证压缩不降列
        page.set_viewport_size({"width": 900, "height": 900})
        page.wait_for_timeout(500)
        screenshot("03-specs-narrow", page)

        # 打印实际列数与列宽，便于断言
        cols = page.evaluate("""() => {
            const grids = [...document.querySelectorAll('.rel-spec-grid')];
            if (!grids.length) return null;
            return grids.map(g => {
                const style = getComputedStyle(g);
                return { cols: style.gridTemplateColumns.split(' ').length,
                         template: style.gridTemplateColumns,
                         cards: g.querySelectorAll('.rel-spec-item').length };
            });
        }""")
        print("layout:", json.dumps(cols, ensure_ascii=False))


if __name__ == "__main__":
    main()
