# DeepSeek Harness 接入 Trellis Card

让 Trellis Card 观察 **DeepSeek Harness（dsh）** 的会话与工具活动，与 Codex / Claude Code 等 Agent 一样显示在卡片里。

## 前置条件

- 已安装 Trellis Card（含 DeepSeek Harness 接入的版本）
- 已安装 dsh CLI：

```bash
npm install -g @deepseek-ai/dsh
```

安装后确认：

```bash
dsh --version   # 应输出 0.1.0-rc.x
```

## 安装步骤

1. 打开 Trellis Card，点卡片右上角三横线 → **设置**
2. 在「Agent 接入」列表找到 **DeepSeek Harness**（第 6 项，全称展示）
3. 点击右侧 **安装**
4. **重启 DeepSeek Harness**（bridge 是 dsh 插件，需要重启服务生效）

安装完成后，设置页该行会显示「已安装」。dsh 在 Trellis 项目中的会话开始、用户 prompt、工具调用会自动出现在卡片里。

## 移除

设置页 DeepSeek Harness 行点 **移除**，会从 dsh 卸载 bridge 插件并删除插件目录。同样重启 dsh 后完全生效。

## 工作原理

dsh 是 cordis 插件体系，**没有 hook 文件机制**，与 Codex / Claude 的接入方式不同。Trellis Card 通过一个内置的 **dsh-trellis-bridge** cordis 插件接入：

```
dsh 会话事件（session/event）
   ↓  bridge 插件订阅（dsh 宿主进程内）
dsh-trellis-bridge 插件
   ↓  调用 trellis-card hook --agent dsh
Trellis Card（socket / inbox 通道）
   ↓  前端展示
卡片 / 胶囊
```

- **bridge 插件**：订阅 dsh 的 `session/event` 事件流，把会话开始、用户 prompt、工具调用映射为 Trellis Card 的事件，再调用本机的 `trellis-card hook --agent dsh` 投递。
- **纯观察者**：只采集会话与工具活动，不参与 dsh 的权限决策，不会阻断或修改任何命令。
- **只观察 Trellis 项目**：只有工作目录（或其祖先）含 `.trellis/tasks` 的会话才会被转发，普通项目静默跳过。
- **安装位置**：bridge 插件复制到 `~/.config/trellis-card/agents/dsh-trellis-bridge/`，并挂载到 dsh web profile（`dsh plugin --profile web add link:<该目录>`）。

## 验证是否生效

1. 确认 bridge 已挂载到 dsh：

```bash
cat ~/.dsh/profiles/web/package.json | grep dsh-trellis-bridge
# 应输出 "dsh-trellis-bridge": "link:/Users/<你>/.config/trellis-card/agents/dsh-trellis-bridge"
```

2. 在一个 Trellis 项目里用 dsh 干活，切回 Trellis Card 看是否有活动出现。

## 常见问题

| 问题 | 处理 |
|---|---|
| 安装时报「无法执行 dsh CLI」 | dsh 未安装，先执行 `npm install -g @deepseek-ai/dsh` |
| 安装了但卡片没活动 | 重启 dsh 让 bridge 生效；确认会话发生在 `.trellis/tasks` 存在的项目里 |
| 其他 Agent 接入不受影响 | dsh 的安装/卸载只操作 `~/.config/trellis-card/agents/` 与 dsh profile，不改动其他 Agent 配置 |
