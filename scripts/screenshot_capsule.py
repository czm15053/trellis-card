#!/usr/bin/env python3
"""
用 Playwright 连接 tauri dev 的本地前端，注入 mock 数据和 Tauri API，
截图验证 card / capsule 紧凑 / capsule 展开 / capsule 菜单 四种状态。
"""
import json
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

DEV_URL = "http://127.0.0.1:1430/"
OUT_DIR = Path(__file__).resolve().parent.parent / "docs" / "screenshots" / "dynamic-island-review"
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
        "subtasks": [
            {"name": "设计登录接口", "status": "completed"},
            {"name": "实现密码加密", "status": "completed"},
            {"name": "前端表单联调", "status": "in_progress"},
            {"name": "编写回归测试", "status": "pending"},
        ],
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
            "prd": True,
            "design": True,
            "implement": True,
            "researchCount": 2,
            "implEntries": 12,
            "checkEntries": 3,
            "reportCount": 0,
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
        "agent": {
            "sessionId": "sess-claude-001",
            "agentKind": "claude",
            "project": project_name,
            "taskId": task_id,
            "eventName": "postToolUse",
            "state": "working",
            "waitingReason": None,
            "toolName": "Bash",
            "toolInput": {"command": "cargo test", "filePath": None},
            "activity": "运行测试中…",
            "startedAt": now - 300,
            "updatedAt": now,
        },
        "activity": "运行测试中…",
        "focusScore": 95,
        "lastChangedAt": now,
    }

    snapshot = {
        "tasks": [runtime_view],
        "projectActivities": [],
        "errors": [],
        "focusKey": task_key,
        "generatedAt": now,
    }

    hooks = [
        {"agent": "codex", "installed": False, "configPath": "~/.codex/hooks.json"},
        {"agent": "claude", "installed": True, "configPath": "~/.claude/settings.json"},
        {"agent": "cursor", "installed": False, "configPath": "~/.cursor/hooks.json"},
        {"agent": "pi", "installed": False, "configPath": "~/.pi/agent/extensions/trellis-card.ts"},
        {"agent": "opencode", "installed": False, "configPath": "~/.config/opencode/plugins/trellis-card.js"},
    ]

    handlers = {
        "complete_setup": {},
        "list_projects": [project],
        "list_tasks": {"version": "v1", "tasks": [task], "errors": []},
        "list_relations": {"tasks": [], "specGroups": []},
        "list_specs": [],
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
        context = browser.new_context(viewport={"width": 1200, "height": 900})
        page = context.new_page()
        page.add_init_script(init_script)
        page.goto(DEV_URL)

        # 等待 setup 页面渲染
        page.wait_for_selector("#btnStartEmpty", timeout=10000)
        page.click("#btnStartEmpty")

        # 等待主界面渲染完成；从零开始会打开 hook 设置面板，先关掉它拍卡片正面
        page.wait_for_selector("#mainView", state="visible", timeout=10000)
        page.wait_for_timeout(800)
        admin_close = page.locator("#admin .list-h button[title='关闭'], #admin .list-h .iconbtn").first
        if admin_close.is_visible():
            admin_close.click()
            page.wait_for_timeout(300)
        # 如果还有设置 scrim，点 scrim 关
        scrim = page.locator("#menuScrim")
        if scrim.is_visible():
            scrim.click()
            page.wait_for_timeout(200)
        screenshot("01-card", page)

        # 切换到胶囊模式
        page.click("#btnMenu")
        page.wait_for_timeout(200)
        page.click("#btnCapsule")
        page.wait_for_timeout(600)
        screenshot("02-capsule-compact", page)

        # hover 展开胶囊
        capsule = page.locator("#capsule")
        capsule.hover()
        page.wait_for_timeout(400)
        screenshot("03-capsule-expanded", page)

        # 打开胶囊菜单
        page.click("#btnCapMenu")
        page.wait_for_timeout(400)
        screenshot("04-capsule-menu", page)

        browser.close()


if __name__ == "__main__":
    main()
