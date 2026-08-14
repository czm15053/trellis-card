# DeepSeek Harness（dsh）使用 Trellis 完整教程

从零开始，让 **DeepSeek Harness（dsh）** 用上 Trellis 的两套能力：

1. **原版 Trellis（CLI）**——dsh 会话里获得 Trellis 任务工作流（任务上下文注入、工具拦截）
2. **Trellis Card（桌面观察者）**——dsh 在 Trellis 项目里的活动实时显示在桌面卡片

两条链路都通过 dsh 插件桥接实现，**无需改 Trellis 源码、无需新建平台**。

---

## 环境前提

| 组件 | 版本 | 安装 |
|---|---|---|
| Node.js | ≥22 | nodejs.org |
| dsh（DeepSeek Harness） | 0.1.0-rc.6 | `npm i -g @deepseek-ai/dsh` |
| Trellis CLI | 0.6.x | `npm i -g @mindfoldhq/trellis` |
| Trellis Card | ≥0.2.0（含 dsh 支持） | 桌面应用 |
| DeepSeek API key | — | [platform.deepseek.com](https://platform.deepseek.com) |

---

## 阶段 0：安装并初始化 dsh

### 1. 安装 dsh

```bash
npm install -g @deepseek-ai/dsh
dsh --version   # 应输出 0.1.0-rc.x
```

### 2. 首次启动（自动创建 profile）

dsh web 首次启动会**自动创建** `~/.dsh/profiles/web/`（含 `cordis.yml`、`cordis.patch.yml`、`package.json`），并初始化 `~/.dsh/`（sessions、storages）。

```bash
dsh web   # 首次启动，然后 Ctrl-C 停掉（或让它常驻）
```

验证 profile 已建：

```bash
ls ~/.dsh/profiles/web/
# → cordis.yml  cordis.patch.yml  package.json  pnpm-lock.yaml
```

### 3. 配置 DeepSeek API key

**方式 A（推荐）**：写入 dsh 的 credentials 文件 `~/.dsh/.credentials.yaml`：

```bash
echo 'DEEPSEEK_API_KEY: sk-你的key' >> ~/.dsh/.credentials.yaml
chmod 600 ~/.dsh/.credentials.yaml
```

**方式 B**：环境变量（临时，重启终端失效）：

```bash
export DEEPSEEK_API_KEY=sk-你的key
```

> 也支持在 dsh web 的设置页（Models）里配置。

验证 key 有效：

```bash
curl -s https://api.deepseek.com/chat/completions \
  -H "Authorization: Bearer $DEEPSEEK_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"deepseek-chat","messages":[{"role":"user","content":"hi"}],"max_tokens":5}'
# 应返回 "content": "..."（非 401/404）
```

### 4. 验证 dsh 能跑真实 agent

```bash
# 在任意目录跑一个简单任务（headless profile 也是首次启动自动建）
dsh --profile headless "说 hi"
```

首次跑 headless 也会自动建 `~/.dsh/profiles/headless/`。如果 key 配好，会返回真实回复。

---

## 阶段 1：dsh 使用原版 Trellis（实验性，有已知问题）

> ⚠️ **注意**：本方案（`dsh-hooks-claude-code` 桥接复用 Claude hooks）**当前有已知 bug**——
> 实测在 dsh web 会话中会导致 `Cannot read properties of undefined (reading 'prepare')`
> 崩溃（dsh-tools code-mode scheduler 相关）。**不是 Trellis Card 或 dsh-trellis-bridge 的问题**，
> headless 环境（同样含 bridge）运行正常。此部分供参考，正式使用建议等 dsh 上游修复。

### 原理

dsh 原生带 **`dsh-hooks-claude-code`** 桥接插件，理论上能消费 **Claude Code 的 hook 配置**，让 Trellis 给 Claude Code 写的 `.claude/settings.json` hooks 在 dsh 会话里生效。

```
Trellis 给 Claude 写的 hooks（.claude/settings.json）
   ↓ dsh 加载 hooks-claude-code 桥接，读取同一份配置
dsh 会话（session-start / pre-step / pre-execute 扩展点触发）
   ↓ 执行 Trellis hook 脚本（session-start.py 等）
dsh 会话获得 Trellis 任务上下文
```

> **已知问题**：dsh 的 `agent/pre-step` 在每个 step（含工具调用后的 step）都执行
> UserPromptSubmit hook，而 Claude Code 只在用户提交时触发。注入的上下文可能破坏
> `tool_calls`/`tool_result` 配对，导致 Anthropic API 报错或 dsh 崩溃。

### 1. 先有 Trellis 项目

在一个项目里初始化 Trellis（若还没有）：

```bash
cd /path/to/project
trellis init --claude   # 给 Claude 平台生成 .claude/settings.json + hooks
```

这会生成：
- `.claude/settings.json`（含 SessionStart / PreToolUse / UserPromptSubmit hooks）
- `.claude/hooks/*.py`（session-start.py、inject-workflow-state.py 等）
- `.trellis/`（tasks、spec、scripts 等）

### 2. 安装 hooks-claude-code 桥接（实验）

```bash
dsh plugin --profile web add @deepseek-ai/dsh-hooks-claude-code@0.1.0-rc.6
# 装 peer 依赖（缺了会报 Cannot find package）
dsh plugin --profile web add \
  @deepseek-ai/dsh-hook-protocol@0.1.0-rc.6 \
  @deepseek-ai/dsh-session-persistence@0.1.0-rc.6 \
  @deepseek-ai/dsh-tools@0.1.0-rc.6 \
  @deepseek-ai/dsh-subagent@0.1.0-rc.6
```

> 提示：这些包没有 `dsh.bundle`，作为普通依赖安装（dsh 提示 warning 属正常），通过第 3 步配置激活。

### 3. 配置 dsh 读取 Claude hooks

编辑 `~/.dsh/profiles/web/cordis.patch.yml`：

```yaml
# Your patch layer for this dsh profile
- insert:
    - id: hooks-claude-code
      name: '@deepseek-ai/dsh-hooks-claude-code'
      config:
        configPath: /path/to/project/.claude/settings.json
        projectDir: /path/to/project
```

- `configPath`：**必填**。Trellis 给 Claude 写的 `.claude/settings.json`。
- `projectDir`：可选。替换 hook 里的 `$CLAUDE_PROJECT_DIR`；省略时默认用会话工作目录。

### 4. 重启 dsh 并验证

```bash
pkill -f "dsh web"; dsh web

# 确认插件进树（无 "could not load hook config" 警告）
dsh --profile web --dump-config | grep hooks-claude-code
# → - id: hooks-claude-code
```

之后在 Trellis 项目里用 dsh 干活，Trellis 任务上下文自动注入。

---

## 阶段 2：dsh 使用 Trellis Card

### 原理

Trellis Card 通过内置的 **`dsh-trellis-bridge`** 插件观察 dsh。它订阅 dsh 的 `session/event` 事件流，把会话开始、用户 prompt、工具调用映射为 Trellis Card 的事件，投递给桌面卡片显示。

```
dsh 会话活动（session/event：user/message、tool/call、step/start、turn/end…）
   ↓ dsh-trellis-bridge 插件（dsh 宿主进程内订阅）
映射为 Trellis Card HookEvent（SessionStart / UserPromptSubmit / PreToolUse / Stop…）
   ↓ 调用 trellis-card hook --agent dsh
Trellis Card（socket / inbox 通道）
   ↓ 前端渲染
桌面卡片显示 dsh 活动与任务进度
```

### 1. 安装 Trellis Card 应用

下载安装含 dsh 支持的 Trellis Card（≥0.2.0），拖进 `/Applications`。

### 2. 安装 dsh-trellis-bridge

**方式 A（推荐）**：在 Trellis Card 设置里装。

1. 打开 Trellis Card → 右上角三横线 → **设置**
2. 「Agent 接入」列表找到 **DeepSeek Harness**（第 6 项，全称）
3. 点 **安装**（应用会复制 bridge 到 `~/.config/trellis-card/agents/dsh-trellis-bridge/`，并挂载到 dsh web profile）
4. 重启 dsh 生效

**方式 B（手动）**：自己复制 bridge + 挂载。

```bash
# 复制 bridge 插件到用户目录（模板来自 Trellis Card 源码 src-tauri/templates/dsh/）
mkdir -p ~/.config/trellis-card/agents/dsh-trellis-bridge/src
cp <模板>/package.json ~/.config/trellis-card/agents/dsh-trellis-bridge/
cp <模板>/cordis.patch.yml ~/.config/trellis-card/agents/dsh-trellis-bridge/
cp <模板>/src/*.js ~/.config/trellis-card/agents/dsh-trellis-bridge/src/

# 挂载到 dsh（web + headless profile 都装，观察所有会话）
dsh plugin --profile web add link:~/.config/trellis-card/agents/dsh-trellis-bridge
dsh plugin --profile headless add link:~/.config/trellis-card/agents/dsh-trellis-bridge
```

### 3. 配置 Trellis Card 扫描项目

首次启动 Trellis Card 选「扫描已有项目」，选择你的 Trellis 根目录。确认 `~/.config/trellis-card/config.json` 里 `roots` 包含你的 Trellis 项目：

```json
{
  "roots": ["/path/to/your/trellis/project"],
  "initialized": true
}
```

> 若卡在 setup 界面，手动创建 config.json（如上）后重启应用进入主界面。

### 4. 启动 dsh 干活，Trellis Card 实时显示

在 Trellis 项目里用 dsh 干活（web UI 发消息，或 headless），dsh 活动自动出现在 Trellis Card：

- **任务进度刻度条**（来自任务 `task.json` 的 `status`：planning≈10%、in_progress≈50%、review≈85%、completed=100%）
- **Agent 活动**（会话开始、工具名、命令、状态）

### 5. 验证

```bash
# 确认 bridge 已挂载
cat ~/.dsh/profiles/web/package.json | grep dsh-trellis-bridge
cat ~/.dsh/profiles/headless/package.json | grep dsh-trellis-bridge

# 跑一个真实 dsh 任务，观察 Trellis Card
cd /path/to/trellis/project
dsh --profile headless "查看 .trellis 任务并报告"
```

---

## 阶段 3：完整验证（真实链路）

```bash
# 1. 确认环境
dsh --version && trellis --version

# 2. 在 Trellis 项目里跑真实 dsh 任务（产生真实 session/event）
cd /path/to/trellis/project
dsh --profile headless "列出 .trellis/tasks 任务并报告你在做什么"

# 3. 观察输出——bridge 会打印投递日志（若在 debug 版）
#    [dsh-trellis-bridge] spawn SessionStart ...
#    [dsh-trellis-bridge] spawn PreToolUse ...
#    [dsh-trellis-bridge] spawn Stop ...

# 4. Trellis Card 窗口应显示该项目 + 任务进度 + dsh 活动
```

---

## 常见问题

| 问题 | 处理 |
|---|---|
| `dsh` 命令不存在 | `npm i -g @deepseek-ai/dsh`，确认 npm 全局 bin 在 PATH |
| 跑 agent 报 `MISSING_CREDENTIAL` / 缺 key | 配 `~/.dsh/.credentials.yaml` 或 export `DEEPSEEK_API_KEY` |
| `Cannot find package '@deepseek-ai/dsh-hook-protocol'` | hooks-claude-code 缺 peer 依赖，补装 4 个包 |
| `could not load hook config` | `configPath` 路径错或 JSON 无效 |
| `Cannot get property "shell" without inject` | web profile 缺 shell 服务，确认 dsh-base 完整 |
| dsh web 报 `reading 'prepare'` / `tool_calls must be followed` | **dsh web 环境已知 bug**（code-mode scheduler），与 Trellis Card 无关；headless 正常。等 dsh 上游修复 |
| Trellis Card 显示不了 dsh 活动 | ①确认进主界面（config.json 有 roots）；②确认 bridge 挂载且 dsh 重启；③确认会话在含 `.trellis/tasks` 的项目 |
| Trellis Card 卡在 setup 界面 | 手动创建 `~/.config/trellis-card/config.json`（含 roots + initialized: true），重启 |

---

## 参考

- dsh hooks 桥接源码：`github.com/deepseek-ai/deepseek-harness` 的 `packages/hooks/hooks-claude-code/`
- dsh-trellis-bridge 插件源码：Trellis Card 的 `src-tauri/templates/dsh/`
