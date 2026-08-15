# WSL 观察模式

让 **Windows 上运行的 Trellis Card** 观察 **WSL 内 Agent**（Claude Code / Codex / Cursor / Pi / OpenCode / DeepSeek Harness）在 Trellis 项目里的活动。

> 场景：Trellis 项目在 WSL 的 Linux 文件系统里（`/home/<user>/proj`），Agent 也在 WSL 里运行。Trellis Card 跑在 Windows 原生，通过 WSL 的 UNC 路径（`\\wsl$\<distro>\...`）读取项目、安装 Hook、接收活动。

## 适用 / 不适用

| 场景 | 支持 |
|---|---|
| Windows 跑 Card，Agent 在 WSL 里跑 Trellis 项目 | ✅ 本模式 |
| 直接在 WSL 里跑 Linux 版 Card（WSLg） | ✅ 天然支持，无需本模式（配置路径就是标准 Linux 路径） |
| 观察 Windows 本机 Agent | ✅ 默认模式，无需启用 WSL |
| WSL 内 `~` 之外的路径（如 `/mnt/c/...` 上的项目） | ⚠️ 项目在 `/mnt/c` 是 Windows 盘符挂载，直接走默认模式即可；WSL 模式针对 Linux 文件系统项目 |

## 启用方法

1. 打开 Trellis Card，点右上角三横线 → **设置**
2. 在「WSL 观察」区块，选择你的 WSL 发行版（如 `Ubuntu`），或选择「停用」关闭
3. 回到「Agent 接入」，安装你使用的 Agent 的 Hook（如 Codex / Claude Code）
4. **重启对应 Agent**（Claude / Codex 等），让新 Hook 生效

启用后，「Agent 接入」里的配置路径会显示为 `\\wsl$\<发行版>\home\<用户>\...`，即写入 WSL 侧配置；Agent 在 Trellis 项目中的活动会自动出现在卡片里。

## 工作原理

### 配置写入 WSL 侧

启用 WSL 观察后，Hook 安装不再写 `C:\Users\...\.claude\settings.json`，而是写：

- **Claude Code**：`\\wsl$\<distro>\home\<user>\.claude\settings.json`
- **Codex**：`\\wsl$\<distro>\home\<user>\.codex\config.toml` + `hooks.json`
- **Cursor**：`\\wsl$\<distro>\home\<user>\.cursor\hooks.json`
- **Pi**：`\\wsl$\<distro>\home\<user>\.pi\agent\extensions\trellis-card.ts`
- **OpenCode**：`\\wsl$\<distro>\home\<user>\.config\opencode\plugins\trellis-card.js`
- **DeepSeek Harness**：`\\wsl$\<distro>\home\<user>\.config\trellis-card\agents\dsh-trellis-bridge\` + `~/.dsh/profiles/web/package.json`

WSL 内 `HOME` 由 `wsl.exe -d <distro> -e sh -c 'echo $HOME'` 读取（默认 `/home/<user>`，root 用户回退 `/root`）。

### Hook 命令

WSL 内 Agent 触发的 Hook 命令指向 Windows 侧可执行文件，路径用 WSL 挂载形式：

```
"/mnt/c/Program Files/Trellis-Card/trellis-card.exe" hook --agent codex
```

Agent 在 WSL（bash）里 spawn 这个命令，派生出的 Windows 进程能看到 Windows Named Pipe —— 事件经现有 `ipc.rs` 通道实时投递，无需新通道。

### 项目路径双向映射

| 方向 | 示例 |
|---|---|
| Agent 上报（WSL Linux 路径）→ Card 存储（Windows UNC） | `/home/alice/proj` → `\\wsl$\Ubuntu\home\alice\proj` |
| Agent 上报（WSL 挂载盘路径）→ Card 存储（Windows 盘符） | `/mnt/c/Users/alice/proj` → `C:\Users\alice\proj` |
| Card 读取（Windows UNC）→ WSL 执行 | `\\wsl$\Ubuntu\home\alice\proj` → `/home/alice/proj` |

映射是纯字符串函数（`platform.rs` 的 `wsl_unc_from_linux` / `linux_from_wsl_unc` / `wsl_mount_to_windows`），不依赖文件系统。

> 注意 `/mnt/c/...` 特例：它表示 Windows 盘在 WSL 里的挂载，Windows 侧直接访问盘符即可，不绕 UNC（`\\wsl$\<distro>\mnt\c\...` 是两层跳转、非规范）。该映射对齐上游 Trellis 的 `_normalize_windows_shell_path`。

### 归档（task.py archive）

`\\wsl$\` 路径的 `task.py` 无法由 Windows Python 直接执行，改经 `wsl.exe` 调用：

```
wsl.exe -d <distro> --cd /home/alice/proj python3 /home/alice/proj/.trellis/scripts/task.py archive <task>
```

## 配置存储

- **WSL 发行版**：应用配置 `config.json` 的 `wsl_distro` 字段，通过设置面板写入
- **覆盖 env**：`TRELLIS_CARD_WSL_DISTRO` 环境变量优先于 GUI 设置（用于脚本/自动化的显式指定）
- 测试 env：`TRELLIS_CARD_WSL_HOME`（注入 WSL home UNC）、`TRELLIS_CARD_WSL_DISTROS`（注入发行版列表）

## 已知限制

- **WSL 发行版枚举**：通过 `wsl.exe -l -q` 检测，仅 Windows 生效；需已安装 WSL 及发行版
- **文件监听**：9P 文件系统上 `notify` 实时监听不可靠，依赖已有轮询兜底（250ms inbox 队列 + 5s 前端轮询）
- **测试环境**：本机为 macOS，Windows 专属代码（`wsl.exe` 调用、Named Pipe 跨 WSL 投递）需在 Windows 上实测验证
- **DeepSeek Harness（DSH）**：DSH bridge 插件需在 dsh 宿主进程内运行，而 dsh 在 WSL 内时，插件安装（`dsh plugin add`）与 profile 读取需经 `wsl.exe` 封装。当前版本未覆盖 WSL 内的 DSH 接入，属已知边界；Claude Code / Codex / Cursor / Pi / OpenCode 均已支持

## 验证是否生效

1. 设置里选择发行版后，「Agent 接入」的配置路径应显示 `\\wsl$\<distro>\...`
2. 安装 Hook 后，检查 WSL 内配置文件确实被写入：

```bash
cat /home/<user>/.codex/hooks.json        # 应包含 trellis-card 的 hook
cat /home/<user>/.claude/settings.json    # 应包含 trellis-card 的 hook
```

3. 在 WSL 内 Trellis 项目里用 Agent 干活，切回 Trellis Card 看是否有活动出现。
