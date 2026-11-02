# Trellis Card 界面设计与交互审查报告 (UI/UX Review & Refactor RFC)

> **评审对象**：Trellis Card 桌面端悬浮挂件与观察者应用（Tauri + Webkit/Chromium）  
> **涉及核心模块**：主卡片（Card Mode）、灵动岛胶囊（Capsule Mode）、任务树（Tree Sheet）、关系与规范地图（Relations Sheet）、控制设置（Admin/Settings Popover）  
> **涉及核心源码**：`src/index.html` (DOM结构), `src/styles.css` (基础排版与组件), `src/app.js` (交互事件与窗口适配), `src/theme-variants.css` (22款主题变体)  
> **审查基准**：桌面微挂件人机工程学、WCAG 2.1 AA 无障碍标准、设计系统一致性、信息架构与视线流规律  
> **文档定位**：由 UI/UX 审计诊断报告升级为**工程可直接落地的重构设计规范（RFC）**

---

## 一、 整体定位与设计基因评估

### 1.1 核心亮点与产品辨识度
* **极具记忆点的视觉隐喻**：将「田野标本卡」（植物藤蔓骨架、衬线题头、古朴纸质微噪点）与「现代遥测控制台」（生命周期状态灯、单行终端流、等宽数据读数）结合，在当前泛滥的浅平同质化开发工具中独树一帜。
* **贴合场景的双模态切换**：
  * **主卡片（344px 宽度）**：面向深度聚焦、检查 PRD 摘要、架构规范与子任务推进；
  * **胶囊态（40px 紧凑 / 136px 展开）**：面向日常编码时的无感陪伴、单行活动播报与轻量知会。
* **清晰的语义色彩映射**：
  * `plan`（紫）/ `work`（琥珀）/ `wrap`（橙）/ `done`（薄荷绿）/ `halt`（玫瑰红）形成闭环的生命周期语义。

### 1.2 核心矛盾点与交互症结
* **诗意标本卡 vs 高密仪器表盘的互搏**：
  * 标本卡追求典雅、呼吸感与大号衬线字体；
  * 仪器表盘追求极致信息密度、等宽微型标签与高频刻度刷新；
  * 当前设计在仅 **344px** 宽的卡片正面塞入了过多层级的元素，导致视线无处落脚，形成了**「遥测数据过载（Telemetry Saturation）」**。
* **物理窗口动态抽搐 vs 悬浮挂件稳定性需求**：
  * 桌面悬浮挂件的核心原则是「静止时如壁纸般稳定，唤起交互时响应灵敏」；
  * 当前应用在胶囊掠过、展开子任务、切换长短任务时频繁调用底层 API 重设系统原生窗口高度，破坏了挂件的物理安全感。

---

## 二、 核心代码资产与架构现状映射

| 模块文件 | 现有代码体积 | 承担角色 | 现状评估与问题归属 |
| :--- | :--- | :--- | :--- |
| `src/index.html` | ~17 KB | 核心 DOM 骨架与各视图 Sheet | 顶栏分为 `.card-grab` 和 `.card-topbar` 两层，结构冗余；缺少显式翻面与置顶入口。 |
| `src/styles.css` | ~98 KB | 基础排版、组件样式、微型状态 | 散落 20 余种 `<10px` 的极小字号；存在大量未变量化的 Hex/RGBA 硬编码；暗灰 `--dim` 对比度不合规。 |
| `src/app.js` | ~193 KB | 窗口通信、视图渲染、事件流控制 | `pointerover` 即时弹开胶囊无防抖延时；`scheduleWindowFit()` 触发频次过高且缺乏视口阻尼；卡片翻面点击过滤规则过严。 |
| `src/theme-variants.css` | ~57 KB | 22 款主题样式覆写 | 主题数量过剩（22款），但二级弹窗（`.menu-pop`）、关联关系图（SVG）与 Markdown 详情大量漏标漏配。 |

---

## 三、 系统性问题深度剖析与根因定位

### 1. 信息层级与空间布局问题 (Hierarchy & Layout)

#### 1.1 顶栏 40px 内存在严重的信息与组件冗余 (ISS-02)
* **代码定位**：`src/index.html:91-112`
* **现状现象**：
  * 拖动栏 `.card-grab` (高 34px)：居中显示纯文本项目名 `.grab-project`（如 `全部项目` / `TRELLIS-CARD`）；
  * 功能栏 `.card-topbar` (高 32px)：左侧紧接着又是一个下拉选择器 `.proj-chip`（同样显示 `全部项目 ▾`）；
* **根因与影响**：
  * 垂直不到 40px 内，完全相同的项目名出现两次；
  * 浪费约 34px 的宝贵纵向视口，且分散用户首屏注意力；
  * 高频的「窗口置顶 (Pin)」与「最小化 (Capsule)」被深藏在右侧二级汉堡菜单中。

#### 1.2 多达 4 重进度指示器同时抢夺视线 (ISS-03)
* **代码定位**：
  1. `src/index.html:97`：顶栏右侧 `.grab-meter`（4 格倾斜刻度 + `1/4` 分数）；
  2. `src/styles.css:487`：左侧藤蔓骨架 `.post`（纵向生长线 + 5 颗发光宝石档位）；
  3. `src/styles.css:761`：英雄舱内 `.runtime-progress`（如 `开发 45% · 子任务 2/5`）；
  4. `src/styles.css:867`：底部子任务分段条 `.subs .segs`（横向色块进度条 + 分数）。
* **影响**：用户无法在 0.5 秒内判断卡片的宏观与微观进展，视觉锚点在左侧、右上、中部和底部之间来回跳跃，产生严重的认知负载。

#### 1.3 正面主卡元素垂直堆叠过密
* **代码定位**：`src/index.html:87-184`
* **现状现象**：344px 宽度的单屏内垂直罗列了：Title、3 项时间线、Hero 状态舱（LED+Icon+状态文本+副文本）、Phase 阶段行、Meta 芯片行（Priority+Workflow+Branch）、Mini Artifacts 读数带、PRD 摘要、子任务列表、Footer 状态字。
* **影响**：卡片内部缺乏负空间（Negative Space/留白），视觉呼吸感不足。

---

### 2. 排版系统与字体层级问题 (Typography & Legibility)

#### 2.1 严重的极端微字（< 10px）泛滥 (ISS-01)
* **代码定位**：`src/styles.css` 中检索到的字号声明
* **数据核查**：界面中散落了 **20 余种零散字号**，大量使用了人眼生理识别困难的亚极限字号：
  * **`7px`**：`.archived-tag`、`.rel-link-card-meta`
  * **`7.5px`**：`.mp-label`、`.rung .lab`、`.rel-spec-cat`、`.rel-spec-status`、`.rel-spec-lines`、`.dev-badge`、`.spec-chip`、`.rel-board-batch-tag`
  * **`8px`**：`.card-status`、`.quote-label`、`.b-label`、`.rel-group-title`、`.rel-track-label`、`.rel-track-count`、`.rel-spec-preview-btn`
  * **`8.5px`**：`.cap-preview-top`、`.observe-timeline li`、`.arts-mini`、`.subline .hint`、`.fid`、`.list-h`、`.dtab`、`.focus-control`、`.hook-state`
  * **`9px` / `9.5px`**：`.cap-project`、`.cap-badge`、`.flipbtn`、`.runtime-activity`、`.when`、`.chip.workflow-chip`
* **危害分析**：
  * 在 1080P/2K 显示器或无视网膜屏（Retina）缩放的屏幕上，**7px ~ 8.5px 文字仅占几个物理点阵**，严重发虚或成马赛克；
  * 严重违反 WCAG 2.1 文本无障碍标准（桌面端最小正文基线 12px~13px，次级徽标基线 10px）。

#### 2.2 字体风格撕裂与暗色对比度损耗
* **代码定位**：`src/styles.css:12-25`
* **现状现象**：标题采用古典衬线体（`ui-serif`），标签采用等宽体（`ui-monospace`），控件采用无衬线体（`system-ui`）；
* 衬线体笔锋在小字号与暗底发光下极易毛糙发灰，缺乏现代 Agent 工具的干练感。

---

### 3. 色彩对比度与多主题支持缺陷 (Colors & Theming)

#### 3.1 暗色模式下的低对比度陷阱 (ISS-04)
* **代码定位**：`src/styles.css:26-35`
* **数据实测**：
  * 核心暗灰变量 `--dim: #63656e` 在主背景 `--ink: #08090d` 上的实际对比度仅为 **3.46:1**，未达到 WCAG AA 级正文的 **4.5:1** 标准；
  * 源码中多处追加 `opacity: .5` / `opacity: .6`（如时间戳、空态提示、已归档状态），实际对比度跌破 **2.0:1**，强光下几乎隐形。

#### 3.2 硬编码色彩破坏主题隔离
* **代码定位**：`src/styles.css` 多处写死 Hex/RGBA
  * `#e7c56a`（waiting permission）
  * `#f07178`（blocked / failed）
  * `#c678dd`（workflow chip）
  * `#4aa3ff`（spec chip）
  * `#8fe3c6`（live badge）
  * `rgba(20,23,33,.92)`（弹出层背景）
* **影响**：切换至 `minimalism`、`swiss`、`wabi-sabi` 等浅色主题时，硬编码的背景和文字依然保持暗色或杂色，导致文字与底色融为一体（例如白底上显示浅黄字）。

#### 3.3 主题数量过剩（22款）但深度不足 (ISS-07)
* 22 款主题表面丰富，但超过 16 款仅调整了边框和极少数背景色；
* 二级面板（Relations 图谱 SVG、设置页 Hook 探测卡片、规范 Markdown 详情）存在大面积漏标漏配。

---

### 4. 桌面挂件交互工效问题 (Widget Ergonomics)

#### 4.1 窗口尺寸随内容动态展开的物理抖动 (ISS-05)
* **代码定位**：`src/app.js:1665` (`scheduleWindowFit()`)
* **现状现象**：
  * 展开/收起子任务、切换长短任务、异步加载 PRD 时，频繁触发 `sendFitWindow(cached)` 调整系统原生窗口尺寸；
  * 桌面原生窗口物理边界频繁跳变，用户产生强烈的视觉疲劳和不可控感。

#### 4.2 灵动岛胶囊 Hover 展开的频闪与误触
* **代码定位**：`src/app.js:4065-4085`, `src/styles.css:348`
* **现状现象**：
  * `cap.addEventListener('pointerover', expand);` **毫无防抖延时（0ms）**；
  * 鼠标只是滑动掠过胶囊时，胶囊瞬间从 40px 弹跳展开至 136px，并触发 Tauri 原生窗口重设；鼠标移开后延迟 120ms 又骤然缩回，形成频闪误触。

#### 4.3 翻面（Card Flip）的盲区挫败感 (ISS-06)
* **代码定位**：`src/app.js:4080-4110`
* **现状现象**：
  * 允许“点击卡片空白区翻面”，但排除列表包含了 `.subs, button, a, details, .doc, .arts-mini, .dtabs, .sheet, .dragbar, input, .card-topbar, .menu-pop, .project-pop, .excerpt`；
  * 在 344px 紧凑卡片上，排查后剩余的空白区域仅剩零星 2~3px 缝隙，用户随意点击往往无响应，造成“功能失灵”的认知假象。

#### 4.4 悬浮挂件核心操作层级过深
* 桌面挂件两项最高频动作——「**窗口置顶（Pin）**」与「**收起为胶囊（Capsule）**」，目前必须点开右上角汉堡菜单在二级弹窗中操作，需要 3 步点击，路径过长。

---

### 5. 高级视图（Relations & Specs）可读性缺陷 (ISS-08)

#### 5.1 关系图谱缺少路径联动高亮
* **代码定位**：`src/index.html:190-210`, `src/app.js` Relations 渲染逻辑
* **现状现象**：SVG 连线包含实线、虚线、紫色、绿色等多种类型，全靠底部微小的 `.rel-legend` 文字对照；当节点交错时，鼠标 Hover 单个节点没有高亮其上下游关联拓扑路径。

#### 5.2 规范地图详情的「滚动套娃（Scroll Trap）」
* **代码定位**：`src/index.html:200-220`, `src/styles.css`
* **现状现象**：Relations Sheet 本身支持全屏滚动，内部 Markdown 规范详情容器又设置了固定最大高度和 `overflow-y: auto`；内层滚动到底部时会突然劫持外层整体移动，造成强烈的滚动穿透与视口失控。

---

## 四、 问题优先级与影响度评估矩阵

| 编号 | 问题项 | 影响范围 | 严重度 | 改造难度 | 建议优先级 | 交付阶段 |
| :---: | :--- | :--- | :---: | :---: | :---: | :---: |
| **ISS-01** | 7px ~ 8.5px 极端微字泛滥，多处文字不可读 | 全局所有视图与组件 | 🔴 高危 | 中 | **P0（最高）** | Phase 1 |
| **ISS-02** | 顶栏双重项目名冗余，高频 Pin/胶囊 藏太深 | 主卡正面顶部第一屏 | 🔴 高危 | 低 | **P0（最高）** | Phase 1 |
| **ISS-03** | 4 重进度指示器同时展示，信息冲突抢视线 | 顶栏/左侧/中部/底部 | 🟡 中危 | 低 | **P0（最高）** | Phase 1 |
| **ISS-04** | 暗灰字对比度不足 3.5:1，硬编码色彩破坏浅色主题 | 全局样式与 22 套主题 | 🟡 中危 | 中 | **P1（重要）** | Phase 1 |
| **ISS-05** | 灵动岛胶囊掠过误触弹跳，窗口频繁物理变高 | 胶囊态与主卡自适应 | 🟡 中危 | 中 | **P1（重要）** | Phase 2 |
| **ISS-06** | 翻面缺少直观视觉提示，盲点空白体验差 | 卡片翻面与背部详情 | 🟢 低危 | 低 | **P1（重要）** | Phase 2 |
| **ISS-07** | 22 款主题良莠不齐，二级页面样式遗漏 | 主题系统与高级视图 | 🟢 低危 | 高 | **P2（进阶）** | Phase 3 |
| **ISS-08** | 关系图谱无 Hover 链路高亮，规范详情双层滚动 | Relations Sheet 关系页 | 🟢 低危 | 中 | **P2（进阶）** | Phase 3 |

---

## 五、 架构级重构落地工程方案 (Actionable Engineering RFC)

### 方案 1：合并重构「一体化控制顶栏」（解决 ISS-02）

#### 1. 结构重构对比
* **重构前（双层 66px 冗余结构）**：
  ```
  ┌───────────────────────────────────────────────┐
  │ ● (LED)          ALL PROJECTS           [||||]│ ← card-grab (34px, 纯拖拽+静态项目名)
  ├───────────────────────────────────────────────┤
  │ [全部项目 ▾]               [列表] [关联] [≡] [📖]│ ← card-topbar (32px, 实际功能栏)
  └───────────────────────────────────────────────┘
  ```
* **重构后（一体化 38px 紧凑顶栏）**：
  ```
  ┌────────────────────────────────────────────────────────┐
  │ [● 全部项目 ▾]          (无缝拖拽区)     [📋] [🕸️] [📌] [—] [≡]│
  └────────────────────────────────────────────────────────┘
  ```

#### 2. HTML 语义重构代码（替换 `src/index.html:91-112`）
```html
<!-- 新的一体化顶栏：高度 38px，集拖拽、项目切换与高频操作于一体 -->
<header class="card-unified-header flex shrink-0 h-[38px] w-full relative z-[10] items-center px-2 justify-between" data-tauri-drag-region>
  <!-- 左侧：项目切换下拉胶囊（内置综合状态指示点） -->
  <button type="button" class="proj-select-chip" id="btnProjectChip" aria-expanded="false" aria-controls="projectPop" title="切换或筛选项目">
    <span class="proj-status-dot" id="grabLed" aria-hidden="true" data-state="idle"></span>
    <span class="proj-name-text" id="projectChipLabel">全部项目</span>
    <svg class="lucide lucide-caret" width="10" height="10" viewBox="0 0 24 24"><use href="icons.svg#i-chevron-down"/></svg>
  </button>

  <!-- 中段：自然拖拽区域 (自动占满剩余空间，不拦截右键菜单) -->
  <div class="header-drag-space flex-1 h-full min-w-[20px]" data-tauri-drag-region></div>

  <!-- 右侧：高频功能快捷键组 -->
  <nav class="topbar-quick-actions flex items-center gap-1" aria-label="快捷窗口操作">
    <button type="button" class="iconbtn" id="btnTree" title="任务列表 (L)" aria-label="任务列表"><svg width="13" height="13"><use href="icons.svg#i-list"/></svg></button>
    <button type="button" class="iconbtn" id="btnRelations" title="关系与规范地图" aria-label="关系图谱"><svg width="13" height="13"><use href="icons.svg#i-share-2"/></svg></button>
    <button type="button" class="iconbtn pin-btn" id="btnQuickTop" title="始终置顶 (Pin)" aria-label="切换窗口置顶" aria-pressed="false"><svg width="13" height="13"><use href="icons.svg#i-pin"/></svg></button>
    <button type="button" class="iconbtn cap-btn" id="btnQuickCapsule" title="折叠为灵动岛胶囊" aria-label="收起为胶囊"><svg width="13" height="13"><use href="icons.svg#i-minus"/></svg></button>
    <button type="button" class="iconbtn menubtn" id="btnMenu" title="更多设置" aria-label="设置菜单" aria-expanded="false"><svg width="14" height="14"><use href="icons.svg#i-menu"/></svg></button>
  </nav>
</header>
```

---

### 方案 2：建立标准化 4 级桌面字阶与无障碍色彩体系（解决 ISS-01、ISS-04）

#### 1. CSS 变量层级重构（注入 `src/styles.css:root`）
彻底禁止使用 `< 10px` 的微字号，将所有 UI 文本强制收敛为 4 级标准：

```css
:root {
  /* ================= 4 级桌面字阶系统 ================= */
  --text-title:     16px;  /* 任务主标题 (下调以减少折行率) */
  --text-body:      13px;  /* 正文描述、子任务行、核心活动状态 */
  --text-sub:       11px;  /* 辅助信息、阶段说明、时间戳、代码路径 */
  --text-badge:     10px;  /* 极小徽标、单色统计Tag (系统底线字号) */

  /* 行高规范 */
  --lh-tight:       1.25;
  --lh-normal:      1.45;
  --lh-relaxed:     1.6;

  /* ================= WCAG 2.1 AA 合规色彩 ================= */
  /* 主背景色为 #08090d 时，确保所有文字对比度 >= 4.5:1 */
  --chalk:          #f2f3f7;      /* 主标题与高亮字，对比度 16.5:1 */
  --mist:           #b8bac6;      /* 次级正文字，对比度 9.2:1 */
  --dim:            #8a8d9b;      /* 辅助弱化字，从 #63656e 提升至 #8a8d9b，对比度 4.85:1 (符合 AA 级) */
  --dim-subtle:     #525562;      /* 仅用于纯装饰性线条或纯图标，不承载文字内容 */
}
```

#### 2. 微字号选择器重构映射表
| 原选择器 | 原字号 | 调整后规范字号 | 改造效果与原因 |
| :--- | :---: | :---: | :--- |
| `.archived-tag`, `.rel-link-card-meta` | `7px` | `10px` (`--text-badge`) | 恢复可读性，适当调减 padding 防止容器被撑大 |
| `.mp-label`, `.rung .lab`, `.rel-spec-cat`, `.spec-chip` | `7.5px` | `10px` (`--text-badge`) | 统一微徽标与类别标签规范 |
| `.card-status`, `.b-label`, `.rel-group-title`, `.rel-track-label` | `8px` | `11px` (`--text-sub`) | 提升关键状态与字段可读性 |
| `.cap-preview-top`, `.observe-timeline li`, `.arts-mini`, `.dtab` | `8.5px` | `11px` (`--text-sub`) | 解决时间线与胶囊顶部字迹模糊发灰问题 |
| `.cap-project`, `.flipbtn`, `.runtime-activity`, `.when` | `9px` | `11px` (`--text-sub`) | 统一归并为二级辅助文字 |

---

### 方案 3：收敛主卡片双层进度体系（解决 ISS-03）

#### 1. 明确双层职责划分
* **宏观进度（Macro Progress）**：
  * **承载者**：左侧藤蔓骨架（`.post` 与 5 颗发光宝石节点）；
  * **职责**：明确指示当前生命周期阶段：`Phase: Plan → Work → Wrap → Done`。
* **微观进度（Micro Progress）**：
  * **承载者**：卡片底部的子任务分段进度条（`.subs .segs`）；
  * **职责**：明确指示当前执行步数与百分比：`3/5 步骤 · 60%`。

#### 2. 移除与切除清单
* **切除**：彻底移除顶部 Header 的 `.grab-meter` 仪表（4 格倾斜刻度 + `1/4` 分数），顶栏空间交还拖拽与功能按钮；
* **弱化**：移除中段 `.runtime-progress` 中的重复步数文本，改为直接在 Hero 舱右上方显示紧凑的小胶囊（如 `P1 · 60%`）。

---

### 4. 方案 4：平滑灵动岛胶囊防抖与视口稳定（解决 ISS-05）

#### 1. 灵动岛防抖展开状态机（重构 `src/app.js:4065-4085`）
```javascript
// 解决 0ms 掠过误触：增加 220ms 意图延时防抖
let capExpandTimer = null;
let capCollapseTimer = null;

const handleCapsulePointerEnter = () => {
  if (capCollapseTimer) { clearTimeout(capCollapseTimer); capCollapseTimer = null; }
  if (cap.classList.contains('is-expanded')) return;

  // 必须悬停超 220ms 判定为有意查看，才触发展开动画与窗口尺寸变化
  capExpandTimer = setTimeout(() => {
    cap.classList.add('is-expanded');
    syncCapsuleWindow();
    capExpandTimer = null;
  }, 220);
};

const handleCapsulePointerLeave = (e) => {
  if (capExpandTimer) { clearTimeout(capExpandTimer); capExpandTimer = null; }
  if (cap.classList.contains('menu-open')) return;
  if (e.relatedTarget && cap.contains(e.relatedTarget)) return;

  // 移出后设置 200ms 缓冲区
  capCollapseTimer = setTimeout(() => {
    cap.classList.remove('is-expanded');
    syncCapsuleWindow();
    capCollapseTimer = null;
  }, 200);
};

cap.addEventListener('pointerenter', handleCapsulePointerEnter);
cap.addEventListener('pointerleave', handleCapsulePointerLeave);
```

#### 2. 稳定主卡片窗口尺寸（视口阻尼）
* 避免单行文本增减就频繁触发 `window.set_size`；
* 设定主卡片正面高度为推荐基准视口（`min-height: 480px; max-height: 520px;`）；
* 子任务或长描述超出时，采用内部无缝平滑自滚动，杜绝原生桌面窗口物理跳动带来的失重感。

---

### 方案 5：增强翻面（Flip）的可发现性与盲区修复（解决 ISS-06）

#### 1. 新增卡片右下角显式翻面折角药丸
在正面卡片右下角或底部操作行，新增常驻微交互控件：
```html
<button type="button" class="card-flip-pill" id="btnPillFlip" title="翻转至背面查看详情与文档 (F)">
  <span class="flip-pill-text">文档 & 证据</span>
  <svg class="flip-icon" width="11" height="11" viewBox="0 0 24 24"><use href="icons.svg#i-refresh-cw"/></svg>
</button>
```

#### 2. 优化空白区点击检测机制
重构 `src/app.js` 中的翻面判定逻辑，不再使用超长排除器，而是划定明确的**「安全翻转区（Safe Flip Zone）」**（如标题右侧留白区、卡片底部状态区），并提供键盘快捷键（按 `F` 键直接翻面）。

---

### 方案 6：精简并打磨 4 款旗舰主题（解决 ISS-07）

精简现存 22 款良莠不齐的主题，将设计与测试资源聚焦在以下 4 款经由完整测试、全组件（含 Relations Sheet & Markdown）适配的旗舰主题：

```
                    ┌─────────────────────────┐
                    │    4 款旗舰主题设计体系   │
                    └───────────┬─────────────┘
          ┌─────────────┬───────┴───────┬─────────────┐
          ▼             ▼               ▼             ▼
   Specimen Nocturne  Zinc Modern    Cyber Console  Paper & Ink
     (默认夜幕标本)     (现代极简)     (极客终端)     (雅致米白浅色)
```

| 主题名称 | 风格定位 | 背景底色 | 主前景色 | 强调色 | 适用场景 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Specimen Nocturne** | 默认标本卡，磨砂深墨 | `#0e1119` | `#f2f3f7` | `#e5a93c` (琥珀) | 沉浸式编码、夜间工作 |
| **Zinc Modern** | 类似 Shadcn / Linear 风格 | `#09090b` | `#fafafa` | `#3b82f6` (科技蓝) | 极简现代化工作流 |
| **Cyber Console** | 纯等宽 CLI 终端黑客风格 | `#0d1117` | `#c9d1d9` | `#39d353` (荧光绿) | 终端重度用户 |
| **Paper & Ink** | 典雅纸质白底黑字（浅色） | `#f7f6f2` | `#1a1c20` | `#a3422a` (朱砂红) | 强光环境、明亮桌面 |

* **浅色主题特别适配机制**：
  * 在 `Paper & Ink` 下，将所有浮层背景强制设为米白色（`#ffffff` / `#eceae4`），阴影由纯黑半透改为柔和环境光晕（`rgba(0,0,0,0.08)`）；
  * 彻底清除未走 CSS 变量的 Hex 硬编码，确保浅色模式下无隐形文本。

---

### 方案 7：高级视图与 Relations 滚动穿透隔离（解决 ISS-08）

#### 1. 关系图谱 SVG Hover 链路双向高亮
* 当鼠标悬停在图谱中的某个节点（`.rel-tnode`）或连线时：
  * 该节点所有上游依赖节点与连线赋予 `.is-ancestor` 类（高亮绿色）；
  * 该节点所有下游依赖节点与连线赋予 `.is-descendant` 类（高亮紫色）；
  * 其余无关节点和连线降低透明度至 `opacity: 0.15`，一秒看清关联脉络。

#### 2. 彻底解决 Spec 详情的「滚动套娃（Scroll Trap）」
在 `src/styles.css` 的 Spec Markdown 滚动容器中加入 CSS 属性：
```css
.rel-spec-detail-content {
  max-height: 320px;
  overflow-y: auto;
  overscroll-behavior: contain; /* 关键：阻止滚动链冒泡，滚动到底部时不驱动外层 Sheet */
}
```

---

## 六、 质量验收与工程规范门禁 (DoD: Definition of Done)

在后续执行代码修改时，必须通过以下验收门禁：

1. **字号硬性门禁（Zero Tolerance for <10px）**：
   * 运行正则扫描 `grep -E 'font-size:\s*(7|7\.5|8|8\.5|9|9\.5)px' src/styles.css`；
   * 验收结果必须为 **0 命中**。
2. **色彩无障碍门禁（WCAG 2.1 AA）**：
   * 在默认夜幕与浅色米白主题下，使用无障碍对比度工具测试主要正文，文本/背景比值必须 **>= 4.5:1**。
3. **顶栏空间与交互门禁**：
   * 主卡片顶栏高度压缩至 **38px**，项目名称仅出现一次；
   * 窗口置顶（Pin）与胶囊收起（Capsule）在顶栏一键直达。
4. **防抖响应门禁**：
   * 鼠标以 >400px/s 速度掠过灵动岛胶囊时，胶囊**不得**展开或触发原生窗口尺寸变更；
   * 鼠标在胶囊上悬停停留持续时间 >220ms 时，胶囊才顺滑展开。

---

## 七、 实施建议路线图（对应后续 Trellis 任务拆解）

本重构建议在后续通过 Trellis 建立正规任务进行追踪推进，推荐拆分为 3 个阶段性子任务（Subtasks）：

```
[UI/UX 重构总任务: ui-ux-refactor]
  ├── 子任务 1: P0 视觉减负与基线规整 (顶栏合并 + 4级字阶 + 进度收敛)
  ├── 子任务 2: P1 交互防抖与视口稳定 (胶囊展开防抖 + 窗口阻尼 + 翻面优化)
  └── 子任务 3: P2 旗舰主题与关系图精修 (4大主题深度适配 + SVG链路高亮)
```

1. **第一阶段：P0 视觉减负与基线规整（预计投入：1-2天）**
   * 合并顶栏为一体化 38px 栏目，释放纵向空间，提权 Pin 与 Capsule；
   * 全面推行 4 级字阶系统，切除 `<10px` 微字，提升 `--dim` 文本明度；
   * 移除顶部 `.grab-meter`，收敛进度为双层结构。
2. **第二阶段：P1 交互防抖与视口稳定（预计投入：2-3天）**
   * 为灵动岛胶囊引入 220ms 意图延时防抖状态机；
   * 稳定主卡片窗口尺寸，采用内部平滑滚动替代窗口物理频繁调整；
   * 增加右下角直观翻面药丸与快捷键。
3. **第三阶段：P2 旗舰主题与关系图精修（预计投入：2-3天）**
   * 精简并彻底适配 4 款旗舰主题（重点攻克 `Paper & Ink` 浅色模式）；
   * 关系图谱支持 Hover 关联拓扑链路高亮；修复 Spec 详情双层滚动穿透。
