# 灵动岛胶囊模式

## 目标

将现有 `capsule` 模式升级为灵动岛风格的紧凑态窗口，同时保留当前卡片模式、状态语义和 Tauri 窗口切换链路。

## 阶段

- [completed] 梳理现有 capsule DOM、状态渲染和窗口模式控制
- [completed] 设计并实现紧凑态灵动岛视觉与展开交互
- [completed] 增加原生窗口收起/展开同步并运行验证
- [completed] 检查工作区差异并总结交付

## 验收标准

- 胶囊模式默认呈现顶部灵动岛式紧凑条，不覆盖整张卡片内容。
- 当前聚焦任务、运行状态、未读数和活动摘要在紧凑态可读。
- 悬停/聚焦可展开更多上下文，点击仍遵循已有胶囊菜单和卡片切换行为。
- 颜色和动效随状态变化，减少动效设置下不产生持续动画。
- `npm test` 和 `npm run check` 在可用环境中通过。

## 已确认事实

- `src/app.js` 已有 `renderCapsule`、`setMode` 和 `set_window_mode` 调用。
- `src/styles.css` 已有 `.capsule`、`.cap-preview`、`.cap-menu-pop` 样式及 `body[data-mode="capsule"]` 布局。
- `src-tauri` 的窗口模式尺寸由后端控制，前端不应新建独立 island DOM 层。

## 实现决策

- Tauri 胶囊窗口使用 `360×62` 收起态和 `360×136` 展开态，避免透明死区阻挡桌面点击。
- `pointerover`、`focusin` 和菜单开启驱动 `128px` 展开态，不增加新的后端状态。
- 灵动岛视觉不继承卡片主题背景，只继承任务状态的 `--accent` 语义色。
