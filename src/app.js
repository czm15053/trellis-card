/* ============================================================
   Trellis Card —— 前端逻辑
   数据全部来自 Tauri invoke（window.__TAURI__.core.invoke）
   ============================================================ */
'use strict';

/* 20 visual themes. The native specimen card is included as the baseline;
   all other entries are adapted from UI-Prompt style families. */
const THEME_DEFS = Object.freeze([
  { id: 'specimen', label: '标本卡', family: 'Trellis native', swatch: ['#0e1119', '#f0a35e'] },
  { id: 'minimalism', label: '极简主义', family: 'Minimalism', swatch: ['#f5f4ef', '#a66522'] },
  { id: 'swiss', label: '瑞士设计', family: 'Swiss Design', swatch: ['#f7f7f4', '#e2463c'] },
  { id: 'industrial', label: '工业设计', family: 'Industrial', swatch: ['#171b1e', '#d99a45'] },
  { id: 'blueprint', label: '蓝图', family: 'Blueprint', swatch: ['#0b2941', '#74d7f2'] },
  { id: 'sci-fi-hud', label: '科幻 HUD', family: 'Sci-Fi HUD', swatch: ['#0b1b1a', '#4be2bc'] },
  { id: 'neon-cyberpunk', label: '霓虹赛博朋克', family: 'Neon Cyberpunk', swatch: ['#130d24', '#ff4f8b'] },
  { id: 'glassmorphism', label: '玻璃态', family: 'Glassmorphism', swatch: ['#121c30', '#72c9ff'] },
  { id: 'soft-ui', label: '软 UI', family: 'Soft UI', swatch: ['#e8eef2', '#6f6bb2'] },
  { id: 'monochrome', label: '黑白单色', family: 'Monochrome', swatch: ['#171717', '#e1e1db'] },
  { id: 'bento', label: '便当盒', family: 'Bento Grids', swatch: ['#faf6ee', '#6b6bd1'] },
  { id: 'brutalism', label: '粗野主义', family: 'Brutalism', swatch: ['#f4ecd8', '#db4448'] },
  { id: 'neo-brutalism', label: '新粗野主义', family: 'Neo Brutalism', swatch: ['#e4dbff', '#ec4d88'] },
  { id: 'memphis', label: '孟菲斯', family: 'Memphis', swatch: ['#fff0dc', '#e85d91'] },
  { id: 'y2k', label: 'Y2K', family: 'Y2K Era', swatch: ['#d8e1ea', '#00b8d9'] },
  { id: 'synthwave', label: '合成波', family: 'Synthwave', swatch: ['#180e35', '#ff4fb3'] },
  { id: 'arcade-crt', label: '街机 CRT', family: 'Arcade CRT', swatch: ['#08160e', '#69f19c'] },
  { id: 'art-deco', label: '装饰艺术', family: 'Art Deco', swatch: ['#151217', '#d7aa58'] },
  { id: 'bauhaus', label: '包豪斯', family: 'Bauhaus', swatch: ['#f3eddd', '#c93e38'] },
  { id: 'wabi-sabi', label: '侘寂', family: 'Wabi-Sabi', swatch: ['#ebe4d7', '#9d7354'] },
]);
const THEME_IDS = THEME_DEFS.map((theme) => theme.id);
const THEME_BY_ID = new Map(THEME_DEFS.map((theme) => [theme.id, theme]));
const isThemeId = (theme) => THEME_IDS.includes(theme);
const themeLabel = (theme) => (THEME_BY_ID.get(theme) || THEME_BY_ID.get('specimen')).label;

/* ---------- 常量 ---------- */
const LANES = ['规划', '动手', '收束', '完结'];
const KIND_COLOR = { plan: '#8b7cf6', work: '#f0a35e', wrap: '#ff8f5a', done: '#45c4a0', halt: '#f07178' };
const HOOK_AGENTS = Object.freeze([
  { id: 'codex', label: 'Codex', description: '采集 Codex 会话中的任务和工具活动', configPath: '~/.codex/hooks.json' },
  { id: 'claude', label: 'Claude Code', description: '采集 Claude Code 会话中的任务和工具活动', configPath: '~/.claude/settings.json' },
  { id: 'cursor', label: 'Cursor', description: '采集 Cursor 会话中的任务和工具活动', configPath: '~/.cursor/hooks.json' },
]);
/* star 色点取项目内「最紧急」kind：卡住 > 收束 > 动手 > 规划 */
const KIND_URGENCY = { halt: 0, wrap: 1, work: 2, plan: 3, done: 4 };
const PREFS_KEY = 'trellis-card';
const POLL_MS = 5000;    // 数据轮询（文件监听的兜底）

const $ = (id) => document.getElementById(id);
const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
const hasTauri = !!(window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke);

/* ---------- 全局状态 ---------- */
const state = {
  view: 'setup',          // 'setup' | 'main'
  configured: false,
  roots: [],
  alwaysOnTop: false,
  projects: [],           // list_projects 结果
  tasksByProject: {},     // name -> { version, tasks[], errors[] }（tasks 已标注 .project）
  runtimeByTask: new Map(),
  runtimeActivities: [],
  runtimeFocusKey: null,
  autoFollowImportant: true,
  autoFollowChangedAt: 0,
  runtimeUnreadCount: 0,
  runtimeNotice: null,
  lastUnreadCandidateKey: null,      // 最近一次已记未读的候选 key（去重用）
  lastUnreadCandidateStamp: 0,       // 最近一次已记未读候选的 lastChangedAt（去重用）
  unreadEvidenceQueue: [],           // 尚未查看的候选任务 [{ key, stamp }]，按最近活动顺序排列
  evidenceTarget: null,              // 查看活动时的临时展示任务（不改变主焦点 focusKey）
  backActiveKey: null,               // 当前背面展示的任务 key（loadBack 异步回填判断用）
  focusMode: 'auto',
  focusLockUntil: 0,
  focusKey: null,         // '项目名::任务id'
  focusedTaskSnapshot: null, // 最近一次主卡任务快照，用于归档后保留回执
  archiveReceipt: null,   // { key, task, rawActivity, focusMode }
  filter: null,           // 项目筛选（null = 全部）
  treeOpen: false,
  adminOpen: false,
  hookStatuses: [],
  hookStatusLoading: false,
  hookStatusRequested: false,
  hookStatusError: null,
  hookUpdatingAgent: null,
  setupBusy: false,
  menuOpen: false,
  themeOpen: false,
  subOpen: null,          // 子任务清单展开的任务 key（内存态）
  flipped: false,         // 卡片是否翻到背面
  prdCache: null,         // 背面文档缓存 { key, docs, error }，聚焦切换即失效
  docSel: null,           // 背面当前选中的文档名
  mode: 'card',           // 'card' | 'capsule'
  theme: isThemeId(document.body.dataset.theme) ? document.body.dataset.theme : 'specimen',
  showArchived: false,    // 是否显示已归档任务（用于树列表过滤）
};
let indexedTasks = [];    // 树列表当前可见扁平顺序（数字键 1-9 用，收起子项不计入）
let treeCollapsed = new Set();  // 已收起父节点的稳定 key（'项目::任务id'），跨渲染保留
let lastFocusKey = null;  // 上次渲染的聚焦（切换动画用）
let entered = false;      // 卡片是否已完成首次进入动画

/* ---------- 小工具 ---------- */
function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
/* Lucide 图标：内联 SVG（用于 JS 动态渲染处）；静态 HTML 用 icons.svg 精灵 <use>。
   aria-hidden 由调用方决定（图标按钮旁必有文字/aria-label 承载语义）。 */
function icon(name, size = 14) {
  return `<svg class="lucide" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><use href="icons.svg#i-${name}"/></svg>`;
}
function runtimeStateIcon(displayState) {
  switch (displayState) {
    case 'waiting_permission':
    case 'waiting_question': return 'circle-pause';
    case 'blocked': return 'circle-alert';
    case 'failed': return 'circle-x';
    case 'turn_done':
    case 'completed': return 'circle-check';
    case 'working': return 'activity';
    default: return 'eye';
  }
}
function focusLockHtml(manual) {
  return `${icon(manual ? 'unlock' : 'lock', 12)}<span>${manual ? '解除锁定' : '锁定当前任务'}</span>`;
}
function runtimeActivityHtml(text) {
  return `<span class="activity-prefix" aria-hidden="true">${icon('arrow-right', 9)}</span>${esc(text)}`;
}
function displayActivity(activity, toolName, action, fallback, context) {
  const raw = String(activity || '').trim();
  const tool = String(toolName || '').trim();
  /* Trellis 工具/命令有明确文档语义，做精确中文翻译；其它工具保持真实工具名，避免误译。 */
  if (tool) {
    const formatter = window.TrellisActivityDisplay;
    const t = tool.toLowerCase();
    if (t.startsWith('trellis-') || t.startsWith('trellis:')) {
      if (formatter) return formatter.semanticizeActivity(raw, tool, action, context) || tool;
      return tool;
    }
    return tool;
  }
  if (!raw) return fallback || '';
  const formatter = window.TrellisActivityDisplay;
  return formatter ? formatter.semanticizeActivity(raw, toolName, action, context) || raw : raw;
}
function errMsg(e) {
  return typeof e === 'string' ? e : (e && e.message) || String(e);
}
function toast(msg) {
  const el = $('toast');
  el.textContent = msg;
  el.classList.add('show');
  clearTimeout(toast._t);
  toast._t = setTimeout(() => el.classList.remove('show'), 2200);
}
/* aria-live 播报：只用于重要切换或需要介入的状态，不播报每条普通 activity */
function announce(msg) {
  const el = $('runtimeAnnouncer');
  if (!el) return;
  el.textContent = '';
  requestAnimationFrame(() => { el.textContent = msg; });
}
function relTime(m) {
  if (!m) return '';
  const t = typeof m === 'number' ? m : new Date(String(m).replace(' ', 'T')).getTime();
  if (!t || Number.isNaN(t)) return '';
  const sec = Math.max(0, Math.round((Date.now() - t) / 1000));
  if (sec < 60) return '刚刚';
  if (sec < 3600) return Math.floor(sec / 60) + ' 分钟前';
  if (sec < 86400) return Math.floor(sec / 3600) + ' 小时前';
  const days = Math.floor(sec / 86400);
  if (days < 45) return days + ' 天前';
  return new Date(t).toISOString().slice(0, 10);
}
/* 项目列表排序：按最近活动时间（last_activity）倒序；无活动时间的排最后 */
function byRecentActivity(a, b) {
  const ta = a.lastActivity ? new Date(String(a.lastActivity).replace(' ', 'T')).getTime() : 0;
  const tb = b.lastActivity ? new Date(String(b.lastActivity).replace(' ', 'T')).getTime() : 0;
  return (tb || 0) - (ta || 0);
}

/* ---------- Tauri 调用 ---------- */
function call(cmd, args) {
  if (!hasTauri) return Promise.reject(new Error('未在 Tauri 环境中运行'));
  return window.__TAURI__.core.invoke(cmd, args);
}
function report(cmd, e) {
  console.error(`[invoke:${cmd}]`, e);
  toast(errMsg(e));
}

/* ---------- 本地持久化 ---------- */
function loadPrefs() {
  try {
    const p = JSON.parse(localStorage.getItem(PREFS_KEY) || '{}');
    if (typeof p.focusKey === 'string') state.focusKey = p.focusKey;
    if (typeof p.filter === 'string') state.filter = p.filter;
    if (typeof p.treeOpen === 'boolean') state.treeOpen = p.treeOpen;
    if (typeof p.alwaysOnTop === 'boolean') state.alwaysOnTop = p.alwaysOnTop;
    /* 缺失或损坏时回退到 true，不影响现有用户 */
    if (typeof p.autoFollowImportant === 'boolean') state.autoFollowImportant = p.autoFollowImportant;
    if (typeof p.showArchived === 'boolean') state.showArchived = p.showArchived;
    if (isThemeId(p.theme)) state.theme = p.theme;
  } catch { /* 忽略损坏的本地数据 */ }
  const requestedTheme = new URLSearchParams(location.search).get('theme');
  if (isThemeId(requestedTheme)) state.theme = requestedTheme;
  applyTheme(state.theme, false);
}
function savePrefs() {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify({
      focusKey: state.focusKey, filter: state.filter,
      treeOpen: state.treeOpen, alwaysOnTop: state.alwaysOnTop,
      autoFollowImportant: state.autoFollowImportant, theme: state.theme,
      showArchived: state.showArchived,
    }));
  } catch { /* localStorage 不可用时静默 */ }
}

function applyTheme(theme, persist = true) {
  const next = isThemeId(theme) ? theme : 'specimen';
  state.theme = next;
  document.body.dataset.theme = next;
  if (persist) savePrefs();
  syncThemeChrome();
}

/* ---------- 数据派生 ---------- */
/* 归档后 id 可能被新任务复用；完整 archive/... 目录才是归档任务的稳定身份。 */
const keyOf = (t) => t.project + '::' + (t.archived && t.dir ? t.dir : t.id);
const unfinished = (t) => t.status !== 'completed' && t.kind !== 'done';
function runtimeProjectName(view) {
  if (!view) return null;
  const project = state.projects.find(
    (item) => item.path === view.project || item.name === view.project,
  );
  return project ? project.name : view.project;
}
function projectNameForPath(path) {
  if (!path) return null;
  const project = state.projects.find((item) => item.path === path || item.name === path);
  if (project) return project.name;
  // Windows 路径用反斜杠，统一按 / 和 \ 切取最后一段（兼容跨平台）
  const parts = String(path).replace(/[/\\]+$/, '').split(/[/\\]+/);
  return parts[parts.length - 1] || null;
}
/* 由任务 key 解析标题（供播报/展示用，避免暴露内部标识） */
function taskTitleFor(focusKey) {
  const t = findTaskByKey(focusKey);
  return t ? (t.title || t.id) : null;
}
/* 由任务 key 查找任务对象（含 .project/.projectPath 标注） */
function findTaskByKey(focusKey) {
  if (!focusKey) return null;
  for (const bucket of Object.values(state.tasksByProject)) {
    const task = (bucket.tasks || []).find(t => keyOf(t) === focusKey);
    if (task) return task;
  }
  return null;
}
function archivedFocusTask(focusKey = state.focusKey) {
  const task = findTaskByKey(focusKey);
  if (!task || !task.archived) return null;
  return !state.filter || task.project === state.filter ? task : null;
}
function archiveReceiptFor(focusKey = state.focusKey) {
  const receipt = state.archiveReceipt;
  if (!receipt || receipt.key !== focusKey) return null;
  if (state.filter && receipt.task.project !== state.filter) return null;
  return receipt;
}
function holdArchivedFocus(focusKey, runtimeView) {
  const source = findTaskByKey(focusKey)
    || (state.focusedTaskSnapshot && keyOf(state.focusedTaskSnapshot) === focusKey
      ? state.focusedTaskSnapshot
      : null);
  if (!source) return false;
  const task = {
    ...source,
    status: 'completed',
    kind: 'done',
    lane: 3,
    partial: 1,
    progress: 1,
  };
  state.archiveReceipt = {
    key: focusKey,
    task,
    rawActivity: String((runtimeView && runtimeView.activity) || '').trim(),
    focusMode: state.focusMode,
  };
  state.focusedTaskSnapshot = task;
  state.focusKey = focusKey;
  state.flipped = false;
  state.prdCache = null;
  state.docSel = null;
  return true;
}
function continueAfterArchive() {
  if (!state.archiveReceipt) return;
  state.archiveReceipt = null;
  state.focusedTaskSnapshot = null;
  state.focusKey = null;
  state.flipped = false;
  state.prdCache = null;
  state.docSel = null;
  clearRuntimeUnread();
  render();
}
function syncUnreadEvidenceState() {
  const queue = state.unreadEvidenceQueue || [];
  state.runtimeUnreadCount = queue.length;
  if (!queue.length) state.runtimeNotice = null;
}
function clearRuntimeUnread() {
  state.unreadEvidenceQueue = [];
  state.runtimeUnreadCount = 0;
  state.runtimeNotice = null;
}
function recordUnreadEvidence(key, stamp) {
  if (!key) return;
  const queue = state.unreadEvidenceQueue || (state.unreadEvidenceQueue = []);
  const nextStamp = stamp || 0;
  const existing = queue.find(item => item.key === key);
  if (existing) {
    queue.splice(queue.indexOf(existing), 1);
    queue.push({ key, stamp: nextStamp });
  } else {
    queue.push({ key, stamp: nextStamp });
  }
  syncUnreadEvidenceState();
}
function acknowledgeUnreadEvidence(entry) {
  if (!entry) {
    clearRuntimeUnread();
    return;
  }
  const queue = state.unreadEvidenceQueue || [];
  const index = queue.findIndex(item => item.key === entry.key);
  if (index < 0) return;
  /* 活动在异步加载期间又更新过时，保留这次新未读。 */
  if ((queue[index].stamp || 0) !== (entry.stamp || 0)) return;
  queue.splice(index, 1);
  syncUnreadEvidenceState();
}
function latestUnreadEvidence() {
  const queue = state.unreadEvidenceQueue || [];
  for (let i = queue.length - 1; i >= 0; i--) {
    const entry = queue[i];
    const task = findTaskByKey(entry.key);
    if (task) return { entry: { ...entry }, task };
  }
  return null;
}
function isNewSinceAutoFollowChanged(view) {
  if (!view || !state.autoFollowChangedAt) return true;
  return (view.lastChangedAt || 0) * 1000 >= state.autoFollowChangedAt;
}
const DISPLAY_COPY = {
  planning: '规划中', working: '正在实现', waiting_permission: '等待授权',
  waiting_question: '等待你的回答', reviewing: '首轮检查', turn_done: '本轮已完成',
  blocked: '已阻塞', failed: '执行失败', stale: '会话已过期', completed: '已完成', idle: '空闲',
};
const ACTION_COPY = {
  create: '创建任务', brainstorm: '头脑风暴', research: '调研', prd: '整理 PRD',
  context: '补充上下文', activate: '激活任务', implement: '实现', check: '检查',
  rollback: '回滚', break_loop: '打破循环', update_spec: '更新规范', archive: '归档',
};
function runtimeKeyFromView(view) {
  if (!view || !view.taskId) return null;
  const project = state.projects.find(p => p.path === view.project || p.name === view.project);
  return `${project ? project.name : view.project}::${view.taskId}`;
}
function runtimeViewForTask(t) {
  const rt = state.runtimeByTask.get(keyOf(t));
  const fallback = t.status === 'completed' ? 'completed' : t.status === 'blocked' ? 'blocked' : t.status === 'failed' ? 'failed' : t.status === 'review' ? 'reviewing' : t.status === 'planning' ? 'planning' : 'idle';
  const base = rt || { displayState: fallback, phase: t.phase && t.phase.id, taskStatus: t.status, focusScore: 0, confidence: 'high', action: null, activity: null, agent: null, lastChangedAt: Math.floor((t.mtime || 0) / 1000) };
  if (base.agent) return base;
  const projectActivity = projectActivityForTask(t);
  if (!projectActivity) return base;
  const projectState = displayStateForActivity(projectActivity);
  const preserveTaskState = new Set(['blocked', 'failed', 'completed']);
  return {
    ...base,
    displayState: preserveTaskState.has(base.displayState) ? base.displayState : (projectState === 'idle' ? base.displayState : projectState),
    activity: base.activity || projectActivity.activity || projectActivity.toolName,
    agent: projectActivity,
    lastChangedAt: Math.max(base.lastChangedAt || 0, projectActivity.updatedAt || 0),
  };
}
function displayStateForActivity(activity) {
  if (!activity) return 'idle';
  if (activity.state === 'waiting') return activity.waitingReason === 'question' ? 'waiting_question' : 'waiting_permission';
  if (activity.state === 'working') return 'working';
  if (activity.state === 'done') return 'turn_done';
  if (activity.state === 'stale') return 'stale';
  return 'idle';
}
function projectActivityForTask(t) {
  if (!t) return null;
  const projectPath = t.projectPath || t.project;
  const now = Math.floor(Date.now() / 1000);
  return state.runtimeActivities
    .filter((activity) => {
      if (!activity || activity.taskId || !activity.project || !activity.state || activity.state === 'none') return false;
      if (now - (activity.updatedAt || 0) > 390) return false;
      return activity.project === projectPath
        || activity.project === t.project
        || projectNameForPath(activity.project) === t.project;
    })
    .sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0))[0] || null;
}
function projectMatchesFilter(project) {
  if (!state.filter) return true;
  const item = state.projects.find(
    (candidate) => candidate.name === project || candidate.path === project,
  );
  return item ? item.name === state.filter : project === state.filter;
}
function latestProjectActivity() {
  const now = Math.floor(Date.now() / 1000);
  return state.runtimeActivities
    .filter(a => a && a.project && a.state && a.state !== 'none' && now - (a.updatedAt || 0) <= 390)
    .filter(a => projectMatchesFilter(a.project))
    .sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0))[0] || null;
}
function projectActivityFallback(currentTask) {
  const activity = latestProjectActivity();
  if (!activity) return null;
  const views = [...state.runtimeByTask.values()].filter(Boolean);
  const hasTaskSessionOnProject = views.some(
    (view) => view.agent && view.project === activity.project,
  );
  const hasImportantTaskView = views.some(
    (view) => TrellisFocusPolicy.isImportantRuntimeState(view.displayState),
  );
  /* 生产策略：有可关联任务（尤其 blocked/failed/waiting 等重要态）时，项目级会话不能盖主卡。 */
  if (!TrellisFocusPolicy.shouldShowProjectActivity({
    hasTaskCandidate: Boolean(currentTask),
    hasImportantTaskView,
    hasTaskSessionOnProject,
  })) {
    return null;
  }
  return activity;
}
function applyRuntimeSnapshot(snapshot) {
  const next = new Map();
  for (const view of (snapshot && snapshot.tasks) || []) {
    const key = runtimeKeyFromView(view);
    if (key) next.set(key, view);
  }
  /* 完成态自动推进需要「之前」的 displayState：快照整体替换前先记住当前 focus 任务的旧态 */
  const prevFocusKey = state.focusKey;
  const prevFocusView = prevFocusKey ? state.runtimeByTask.get(prevFocusKey) : null;
  const prevFocusState = (prevFocusView && prevFocusView.displayState) || null;
  state.runtimeByTask = next;
  state.runtimeActivities = (snapshot && snapshot.projectActivities) || [];
  const previousRuntimeFocusKey = state.runtimeFocusKey;
  /* focus-task-changed 事件可能漏收；从任务级 Agent 会话补齐焦点。 */
  const liveTaskFocus = [...next.entries()]
    .filter(([, view]) => view && view.agent && view.agent.state && view.agent.state !== 'none')
    .sort(([, a], [, b]) => {
      const score = (b.focusScore || 0) - (a.focusScore || 0);
      return score || ((b.agent && b.agent.updatedAt) || 0) - ((a.agent && a.agent.updatedAt) || 0);
    })[0];
  const focusView = (snapshot && snapshot.focusKey) || (liveTaskFocus && liveTaskFocus[0]) || null;
  if (focusView) {
    const [project, ...id] = String(focusView).split('::');
    const projectInfo = state.projects.find(p => p.path === project || p.name === project);
    if (projectInfo && id.length) {
      state.runtimeFocusKey = `${projectInfo.name}::${id.join('::')}`;
    } else if (liveTaskFocus && liveTaskFocus[0]) {
      state.runtimeFocusKey = liveTaskFocus[0];
    } else {
      state.runtimeFocusKey = String(focusView);
    }
  } else {
    state.runtimeFocusKey = null;
  }
  const currentFocusView = state.runtimeByTask.get(prevFocusKey);
  const archiveAction = (prevFocusView && prevFocusView.action === 'archive')
    || (currentFocusView && currentFocusView.action === 'archive');
  const wasActiveBeforeArchive = prevFocusState
    && prevFocusState !== 'completed'
    && prevFocusState !== 'turn_done';
  const archiveCompleted = Boolean(
    prevFocusKey
      && wasActiveBeforeArchive
      && archiveAction
      && (currentFocusView && currentFocusView.displayState === 'completed' || !currentFocusView),
  );
  if (archiveCompleted) {
    holdArchivedFocus(prevFocusKey, currentFocusView || prevFocusView);
  }
  /* 归档回执存在时，后续实时任务只记录活动，不抢走当前回执。 */
  const followEnabled = state.archiveReceipt ? false : state.autoFollowImportant;
  const now = Date.now();
  const runtimeFocusChanged = state.runtimeFocusKey !== previousRuntimeFocusKey;
  /* 始终更新后端候选；候选按分类决定：adopt（切换焦点）/ refresh-same（原地刷新，不记未读）/ reject（记未读）。 */
  if (state.runtimeFocusKey) {
    const candidateView = state.runtimeByTask.get(state.runtimeFocusKey);
    const classification = TrellisFocusPolicy.classifyRuntimeCandidate({
      enabled: followEnabled,
      locked: state.focusMode === 'manual',
      hasCurrentFocus: Boolean(state.focusKey),
      currentKey: state.focusKey,
      candidateKey: state.runtimeFocusKey,
      nextState: candidateView && candidateView.displayState,
      filter: state.filter,
      nextProject: runtimeProjectName(candidateView),
      isNewSinceEnabled: isNewSinceAutoFollowChanged(candidateView),
    });
    if (classification === 'adopt') {
      if (state.focusKey !== state.runtimeFocusKey) {
        const targetView = candidateView;
        state.focusKey = state.runtimeFocusKey;
        state.flipped = false;
        state.prdCache = null;
        state.docSel = null;
        clearRuntimeUnread();
        /* 已采纳的候选不应再被去重标记占用：同一 key 后续被拒时仍可记未读 */
        state.lastUnreadCandidateKey = null;
        state.lastUnreadCandidateStamp = 0;
        announce(`已切换到任务 ${taskTitleFor(state.focusKey) || state.focusKey}，原因是 ${DISPLAY_COPY[targetView && targetView.displayState] || '实时活动'}`);
      }
    } else if (classification === 'reject') {
      /* 候选被拒绝：只增加未读提示，不改变当前任务或阅读位置。
         未读与自动跟随开关、手动锁定均无关——开关只控制是否切换焦点，
         manual 只阻止采纳，不阻止未读。
         去重：同一候选 key 且 lastChangedAt 未变时（轮询 / 双事件重复），不重复记未读。 */
      const candidateIsDone = candidateView && (candidateView.displayState === 'completed' || candidateView.displayState === 'turn_done');
      const candidateIsCurrent = state.focusKey === state.runtimeFocusKey;
      if (!(candidateIsDone && !candidateIsCurrent)) {
        /* 完成态候选（非当前焦点）：完成推进已把焦点切走，后续 snapshot 指向已完成任务
           不应记未读噪音（classifyRuntimeCandidate 已 reject，这里跳过未读记录）。 */
        const dedup = TrellisFocusPolicy.unreadDedup(
          state.lastUnreadCandidateKey,
          state.lastUnreadCandidateStamp,
          state.runtimeFocusKey,
          candidateView && candidateView.lastChangedAt,
        );
        if (dedup.record) {
          state.runtimeNotice = state.runtimeNotice || '有未查看的实时活动';
          /* 同一任务只占一个未读入口，活动更新时替换其时间戳，避免计数和目标不一致。 */
          state.lastUnreadCandidateKey = dedup.nextKey;
          state.lastUnreadCandidateStamp = dedup.nextStamp;
          recordUnreadEvidence(state.runtimeFocusKey, dedup.nextStamp);
        }
      }
    }
    /* refresh-same：原地刷新，无需动作——runtimeByTask 已更新，render 会展示新状态 */
  }
  if (runtimeFocusChanged && state.focusLockUntil <= now) {
    state.focusLockUntil = 0;
  }
  /* 完成态自动推进：当前 focus 任务刚由非完成态进入 completed/turn_done，且满足策略条件时，
     切换到下一个进行中任务。须在快照应用后且 focusKey 未被 adopt 换掉时触发。 */
  if (state.focusKey === prevFocusKey && prevFocusState) {
    const curView = state.runtimeByTask.get(state.focusKey);
    const curState = curView && curView.displayState;
    const shouldAdvance = TrellisFocusPolicy.shouldAutoAdvanceOnCompletion({
      enabled: followEnabled,
      locked: state.focusMode === 'manual',
      hasCurrentFocus: Boolean(state.focusKey),
      prevState: prevFocusState,
      nextState: curState,
      action: curView && curView.action,
    });
    if (shouldAdvance) {
      if (pool().some(t => keyOf(t) !== state.focusKey && unfinished(t))) {
        const nextKey = TrellisFocusPolicy.nextFocusAfterCompletion({
          candidates: pool()
            .filter(t => unfinished(t))
            .map(t => ({
              key: keyOf(t),
              completed: !unfinished(t),
              /* 最近有变化 = runtime 活动时间；缺失时用任务文件 mtime 兜底 */
              lastChangedAt: Math.max(runtimeViewForTask(t).lastChangedAt || 0, t.mtime || 0),
              mtime: t.mtime || 0,
            })),
          currentKey: state.focusKey,
        });
        if (nextKey && nextKey !== state.focusKey) {
          const oldKey = state.focusKey;
          state.focusKey = nextKey;
          state.flipped = false;
          state.prdCache = null;
          state.docSel = null;
          clearRuntimeUnread();
          state.lastUnreadCandidateKey = null;
          state.lastUnreadCandidateStamp = 0;
          const doneTitle = taskTitleFor(oldKey) || oldKey;
          const nextTitle = taskTitleFor(nextKey) || nextKey;
          toast(`「${doneTitle}」已完成，已切换到「${nextTitle}」`);
          announce(`已完成 ${doneTitle}，已自动切换到任务 ${nextTitle}`);
        }
      }
    }
  }
}
async function refreshRuntimeSnapshot() {
  try {
    const snapshot = await call('get_runtime_snapshot');
    applyRuntimeSnapshot(snapshot || {});
    return true;
  } catch (e) {
    /* runtime 快照不可用时降级到文件扫描，不阻塞主流程。 */
    if (hasTauri) console.error('[invoke:get_runtime_snapshot]', e);
    return false;
  }
}
/* 最近一次 AI 会话活跃时间（epoch ms，无会话为 0） */
function lastSeen(t) {
  let m = 0;
  for (const s of t.sessions || []) {
    const v = Date.parse(s.lastSeenAt) || 0;
    if (v > m) m = v;
  }
  return m;
}
const LIVE_MS = 60 * 60e3;   // 1 小时内有会话视为「活跃中」
const isLive = (t) => lastSeen(t) && (Date.now() - lastSeen(t) < LIVE_MS);

/* 当前筛选下的任务池（不含已归档任务：归档仅查看，不参与焦点/进度/统计） */
function pool() {
  const names = state.filter ? [state.filter] : Object.keys(state.tasksByProject);
  const out = [];
  for (const n of names) {
    const bucket = state.tasksByProject[n];
    if (bucket) out.push(...bucket.tasks.filter(t => !t.archived));
  }
  return out;
}
/* 默认聚焦：有活跃会话的优先（lastSeenAt 新的在前），其余按 mtime 最大 */
function defaultFocus(tasks) {
  const un = tasks.filter(unfinished);
  if (!un.length) return null;
  if (state.focusMode === 'auto' && state.runtimeFocusKey) {
    const runtimeTask = un.find(t => keyOf(t) === state.runtimeFocusKey);
    if (runtimeTask) return runtimeTask;
  }
  const live = un.filter(t => lastSeen(t));
  if (live.length) return live.sort((a, b) => (runtimeViewForTask(b).focusScore || 0) - (runtimeViewForTask(a).focusScore || 0) || lastSeen(b) - lastSeen(a))[0];
  return un.reduce((a, b) => ((b.mtime || 0) > (a.mtime || 0) ? b : a));
}
/* 当前聚焦任务；没有任何未完成任务时返回 null（空闲态）。
   手动锁定指向归档任务时（用户点击列表里的已归档项），从完整数据找回并展示。 */
function currentFocus() {
  const p = pool();
  const archiveReceipt = archiveReceiptFor();
  if (archiveReceipt) return archiveReceipt.task;
  /* 显式锁定优先；归档任务不在活跃池中，需从完整数据单独找回。 */
  if (state.focusMode === 'manual' && state.focusKey) {
    const explicit = p.find(t => keyOf(t) === state.focusKey)
      || archivedFocusTask();
    if (explicit) return explicit;
  }
  if (!p.some(unfinished)) return null;
  /* focusLockUntil 仅作旧偏好兼容，不再悄悄解除用户锁定 */
  return p.find(t => keyOf(t) === state.focusKey) || defaultFocus(p);
}
function ensureFocusValid() {
  if (state.archiveReceipt && !archiveReceiptFor()) {
    state.archiveReceipt = null;
    state.focusedTaskSnapshot = null;
  }
  if (archiveReceiptFor()) return;
  if (!state.focusKey) return;
  /* 手动点击的归档任务不在活跃池中，但仍是有效的只读焦点。轮询刷新时
     不能因此清掉锁定，否则自动跟随会在数秒后跳转到其它运行中的任务。 */
  const manuallyPinned = state.focusMode === 'manual' && archivedFocusTask();
  if (!pool().some(t => keyOf(t) === state.focusKey) && !manuallyPinned) state.focusKey = null;
}
/* 树列表：未完成全保留；已完成默认只留 mtime 最近 3 个。
   勾选「显示已归档」时，全部已归档任务（t.archived）追加显示，普通已完成仍只留最近 3 个。 */
function trimCompleted(tasks, showArchived) {
  const active = tasks.filter(t => !t.archived && unfinished(t));
  const done = tasks.filter(t => !unfinished(t) && !t.archived)
    .sort((a, b) => (b.mtime || 0) - (a.mtime || 0)).slice(0, 3);
  if (showArchived) {
    const archived = tasks.filter(t => t.archived)
      .sort((a, b) => (b.mtime || 0) - (a.mtime || 0));
    return [...active, ...done, ...archived];
  }
  return [...active, ...done];
}
/* 按 parent / children 组树；parent 不在列表中时提升为顶层 */
function buildTree(tasks) {
  const inList = new Set(tasks.map(t => t.id));
  const implied = new Map();   // children 反推的父 id（任务自身未声明 parent 时兜底）
  for (const t of tasks) {
    for (const c of t.children || []) {
      if (inList.has(c) && !implied.has(c)) implied.set(c, t.id);
    }
  }
  const top = [], kids = new Map();
  for (const t of tasks) {
    let pid = (t.parent && inList.has(t.parent)) ? t.parent : null;
    if (!pid) {
      const ip = implied.get(t.id);
      if (ip && inList.has(ip)) pid = ip;
    }
    if (pid) {
      if (!kids.has(pid)) kids.set(pid, []);
      kids.get(pid).push(t);
    } else {
      top.push(t);
    }
  }
  const byMtime = (a, b) => (b.mtime || 0) - (a.mtime || 0);
  top.sort(byMtime);
  kids.forEach(list => list.sort(byMtime));
  return { top, kids };
}

/* ---------- 数据刷新 ---------- */
async function refresh(force = false) {
  let projects;
  try {
    projects = await call('list_projects');
  } catch (e) {
    report('list_projects', e);
    return false;
  }
  state.projects = (projects || []).sort(byRecentActivity);
  let changed = force;
  const seen = new Set();
  for (const p of state.projects) {
    seen.add(p.name);
    let res;
    try {
      res = await call('list_tasks', { project: p.path });
    } catch (e) {
      report('list_tasks', e);
      continue;
    }
    const old = state.tasksByProject[p.name];
    /* version 未变则跳过该项目的重渲染 */
    if (!old || old.version !== res.version) {
      changed = true;
      state.tasksByProject[p.name] = {
        version: res.version,
        errors: res.errors || [],
        tasks: (res.tasks || []).map(t => ({ ...t, project: p.name, projectPath: p.path })),
      };
    }
  }
  for (const n of Object.keys(state.tasksByProject)) {
    if (!seen.has(n)) { delete state.tasksByProject[n]; changed = true; }
  }
  if (state.filter && !state.projects.some(p => p.name === state.filter)) state.filter = null;
  const runtimeChanged = await refreshRuntimeSnapshot();
  changed = changed || runtimeChanged;
  ensureFocusValid();
  updateStatus();
  if (changed && state.view === 'main') render();
  return changed;
}
function updateStatus() {
  const errs = Object.values(state.tasksByProject).reduce((n, b) => n + (b.errors ? b.errors.length : 0), 0);
  const active = pool().filter(unfinished).length;
  const time = new Date().toLocaleTimeString('zh-CN', { hour12: false });
  $('status').textContent =
    `${state.projects.length} 个项目 · ${active} 进行中 · ${time}` + (errs ? ` · ${errs} 条解析错误` : '');
}
async function manualRefresh() {
  await refresh(true);
  render();
  toast('已刷新');
}

/* ---------- 渲染总入口 ---------- */
function render() {
  if (state.view !== 'main') return;
  document.body.dataset.mode = state.mode;
  /* 卡片模式且未翻面：高度随内容自适应（底部透明）；翻面/胶囊保持固定高度 */
  syncFit();
  renderStars();
  const t = currentFocus();
  if (t) state.focusedTaskSnapshot = t;
  const projectActivity = projectActivityFallback(t);
  renderCard(t, projectActivity);
  renderTree(t);
  renderCapsule(t, projectActivity);
  renderAdmin();
  syncTools();
  savePrefs();
  /* sheet 无障碍语义：展开/收起同步 aria-hidden */
  const listSheet = $('list');
  if (listSheet) listSheet.setAttribute('aria-hidden', String(!state.treeOpen));
  const adminSheet = $('admin');
  if (adminSheet) adminSheet.setAttribute('aria-hidden', String(!state.adminOpen));
}

/* ---------- 项目筛选（Tree 顶栏 + Project chip popover，同一 filter） ---------- */
function filterOptions() {
  /* 全部计数不依赖当前 filter，避免 chips 数字随筛选收缩 */
  let allCount = 0;
  for (const p of state.projects) {
    allCount += (state.tasksByProject[p.name]?.tasks || []).filter(unfinished).length;
  }
  const opts = [{ name: null, label: '全部', count: allCount, color: '#a6a7ae' }];
  for (const p of state.projects) {
    const tasks = (state.tasksByProject[p.name]?.tasks || []).filter(unfinished);
    if (!tasks.length && state.filter !== p.name) continue;
    let best = 'plan';
    for (const t of tasks) {
      if ((KIND_URGENCY[t.kind] ?? 9) < (KIND_URGENCY[best] ?? 9)) best = t.kind;
    }
    opts.push({
      name: p.name,
      label: p.name,
      count: tasks.length,
      color: KIND_COLOR[best] || '#a6a7ae',
    });
  }
  return opts;
}
function applyProjectFilter(name) {
  state.filter = name || null;
  ensureFocusValid();
  closeProjectPop();
  render();
}
function fillFilterStars(box, { closePop = false } = {}) {
  if (!box) return;
  box.innerHTML = '';
  for (const opt of filterOptions()) {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'star' + ((opt.name ? state.filter === opt.name : !state.filter) ? ' on' : '');
    b.style.setProperty('--accent', opt.color);
    b.setAttribute('role', 'option');
    b.setAttribute('aria-selected', String(opt.name ? state.filter === opt.name : !state.filter));
    b.innerHTML = `<i style="--c:${opt.color}"></i>${esc(opt.label)}${opt.name ? ` ${opt.count}` : ''}`;
    b.onclick = () => {
      if (closePop) applyProjectFilter(opt.name);
      else {
        state.filter = opt.name || null;
        ensureFocusValid();
        render();
      }
    };
    box.appendChild(b);
  }
}
function renderStars() {
  fillFilterStars($('treeStars'));
  fillFilterStars($('projectPop'), { closePop: true });
  const chipLabel = $('projectChipLabel');
  if (chipLabel) chipLabel.textContent = state.filter ? state.filter : '全部项目';
  const chip = $('btnProjectChip');
  if (chip) chip.classList.toggle('on', !!state.filter || isProjectPopOpen());
}
function isProjectPopOpen() {
  const pop = $('projectPop');
  return !!(pop && !pop.hidden);
}
function closeProjectPop() {
  const pop = $('projectPop');
  const btn = $('btnProjectChip');
  if (pop) pop.hidden = true;
  if (btn) {
    btn.classList.remove('on');
    btn.setAttribute('aria-expanded', 'false');
  }
}
function toggleProjectPop(force) {
  const pop = $('projectPop');
  const btn = $('btnProjectChip');
  if (!pop || !btn || state.mode === 'capsule') return;
  const open = force !== undefined ? force : pop.hidden;
  if (open) {
    /* 开 chip 时关主菜单，避免双 popover */
    if (state.menuOpen) toggleMenu(false);
    fillFilterStars(pop, { closePop: true });
    pop.hidden = false;
    btn.classList.add('on');
    btn.setAttribute('aria-expanded', 'true');
    focusReturnTo = btn;
    setTimeout(() => moveFocusInto(pop), 50);
  } else {
    closeProjectPop();
    restoreFocus();
  }
}

/* ---------- 左侧藤蔓脊 ---------- */
function renderPost(t) {
  const post = $('post');
  post.innerHTML = '';
  const fill = document.createElement('div');
  fill.className = 'post-fill';
  post.appendChild(fill);
  const laneHints = ['需求评审与上下文准备', '开发实现与工具调用', '自测、回归与问题收口', '完成、归档与交付'];
  LANES.forEach((lab, i) => {
    const r = document.createElement('div');
    let cls = 'rung';
    if (t) {
      if (i < (t.lane ?? 0) || t.kind === 'done') cls += ' done';
      else if (i === (t.lane ?? 0)) cls += ' now';
    }
    r.className = cls;
    r.title = `${lab}：${laneHints[i]}`;
    r.dataset.tip = r.title;
    r.setAttribute('aria-label', r.title);
    r.innerHTML = `<div class="jewel"></div><div class="lab">${lab}</div>`;
    post.appendChild(r);
  });
  if (t) {
    const progressed = t.kind === 'done' ? 1 : Math.min(1, ((t.lane ?? 0) + (t.partial || 0)) / 4);
    fill.style.height = `calc((100% - 48px) * ${Math.max(0.05, progressed)})`;
  } else {
    fill.style.height = '0px';
  }
}

/* ---------- 主区：中腰 excerpt → description → 子任务预览；子任务条沉底并存 ---------- */
/* 任务专属指标只属于详情文档，不进入正面观察卡或胶囊的通用信息层。 */
function metricTags(t) {
  const source = String((t && (t.excerpt || t.description)) || '');
  if (!source) return '';
  const tags = [];
  const ttft = source.match(/(?:TTFT|首字延迟)[^。\n]{0,40}?([0-9]+(?:\.[0-9]+)?)\s*ms/i);
  if (ttft) tags.push(`TTFT ≤ ${ttft[1]}ms`);
  const fps = source.match(/([0-9]+(?:\.[0-9]+)?)\s*fps/i);
  if (fps) tags.push(`帧率 ≥ ${fps[1]}fps`);
  if (!tags.length) return '';
  return `<div class="metric-tags" aria-label="任务指标">${tags.map(tag => `<span>${esc(tag)}</span>`).join('')}</div>`;
}

function stageHtml(t, rt) {
  const subs = t.subtasks || [];
  const fill = (inner) => `<div class="stage-fill">${inner}</div>`;
  /* 介入上下文（blocked/failed）：真实 rt.action/activity/phase，填充中腰空白，
     与 hero 的单条 activity 副行不同——这里结构化列出「阻塞/失败时的介入信息」。 */
  const ds = rt && rt.displayState;
  const intervene = ds === 'blocked' || ds === 'failed';
  const interventionHtml = intervene ? `
    <div class="intervene">
      <div class="quote-label">介入上下文</div>
      <div class="iv-row"><span class="iv-key">状态</span><span class="iv-val">${esc(DISPLAY_COPY[ds] || ds)}${t.phase && t.phase.label ? ` · ${esc(t.phase.label)}` : ''}</span></div>
      ${rt.action ? `<div class="iv-row"><span class="iv-key">最近动作</span><span class="iv-val">${esc(ACTION_COPY[rt.action] || rt.action)}</span></div>` : ''}
      ${rt.activity ? `<div class="iv-row"><span class="iv-key">活动</span><span class="iv-val">${esc(rt.activity)}</span></div>` : ''}
    </div>` : '';
  /* 中腰内容：excerpt > description > 子任务前几条预览 > 空提示。
     介入上下文（blocked/failed）追加到内容内部（desc 正下方），
     剩余空间由 stage-fill 弹性吸收到 intervene 之后，避免中部空白。 */
  let midHtml = '';
  if (t.excerpt) {
    midHtml = fill(`<div class="quote-label">PRD · 摘要</div><blockquote class="excerpt">${esc(t.excerpt)}</blockquote>${interventionHtml}`);
  } else if (t.description) {
    midHtml = fill(`<div class="quote-label">任务描述</div><p class="desc">${esc(t.description)}</p>${interventionHtml}`);
  } else if (subs.length) {
    const rows = subs.slice(0, 3).map((s) => {
      const done = s.status === 'completed';
      return `<li class="${done ? 'done' : ''}"><i></i><span>${esc(s.name)}</span></li>`;
    }).join('');
    const more = subs.length > 3 ? `<div class="subprev-more">还有 ${subs.length - 3} 条，见下方清单</div>` : '';
    midHtml = fill(`<div class="quote-label">子任务</div><ul class="subprev">${rows}</ul>${more}`);
  } else {
    midHtml = fill('<p class="desc" style="color:var(--dim)">这个任务还没有留下更多细节。</p>');
  }
  /* 底部：子任务分段条 */
  if (subs.length) {
    const doneN = subs.filter(s => s.status === 'completed').length;
    const curIdx = subs.findIndex(s => s.status !== 'completed');
    const segs = subs.map((s, i) => {
      const cls = s.status === 'completed' ? 'seg f' : (i === curIdx ? 'seg c' : 'seg');
      return `<i class="${cls}"></i>`;
    }).join('');
    const curName = curIdx >= 0 ? subs[curIdx].name : '全部完成';
    const verificationSub = /verify|验证|回归|test|测试|check|gate/i.test(curName);
    const subCaption = verificationSub ? '回归验证进度' : '子任务进度';
    /* 下一个待办：当前之后的第一个未完成 */
    const nextIdx = curIdx >= 0 ? subs.findIndex((s, i) => i > curIdx && s.status !== 'completed') : -1;
    const nextHtml = nextIdx >= 0
      ? `<div class="nextline">下一个 → ${esc(subs[nextIdx].name)}</div>`
      : (curIdx >= 0 ? '<div class="nextline done-all">这是最后一步，做完就收束</div>' : '');
    const open = state.subOpen === keyOf(t);
    const items = subs.map((s, i) => {
      const cls = s.status === 'completed' ? 'done' : (i === curIdx ? 'cur' : '');
      return `<li class="${cls}"><i></i><span>${esc(s.name)}</span></li>`;
    }).join('');
    return midHtml + `
      <div class="subs${open ? ' open' : ''}" id="subsBox" tabindex="0" title="点击展开子任务清单">
        <div class="segs">${segs}</div>
        <div class="sub-caption"><span>${subCaption}</span><span class="hint">${open ? '收起' : '清单'}</span></div>
        <div class="subline">
          <span class="cur">${esc(curName)}</span>
          <span class="n">${doneN}/${subs.length}</span>
        </div>
        ${nextHtml}
        <div class="sublist"><div class="sublist-in"><ul>${items}</ul></div></div>
      </div>${midHtml.includes('class="intervene"') ? '' : interventionHtml}`;
  }
  /* 非 subs 分支：interventionHtml 已并入 midHtml 的 fill 内部（desc/excerpt 正下方） */
  return midHtml;
}
/* artifacts 迷你读数带（单行紧凑，翻面是大读数区） */
function artsMini(t) {
  const a = t.artifacts;
  if (!a) return '';
  const flag = (label, ok) =>
    `<span class="${ok ? 'am-ok' : 'am-no'}">${label} ${ok ? icon('check', 11) : icon('x', 11)}</span>`;
  const cnt = (label, n) =>
    `<span class="${n > 0 ? 'am-ok' : 'am-zero'}">${label} <b>${n}</b></span>`;
  return `<div class="arts-mini">
    ${flag('PRD', !!a.prd)}${flag('DESIGN', !!a.design)}${flag('IMPL', !!a.implement)}
    <span class="am-sep"></span>
    ${cnt('调研', a.researchCount || 0)}${cnt('注入', a.implEntries || 0)}${cnt('检查', a.checkEntries || 0)}${cnt('报告', a.reportCount || 0)}
  </div>`;
}
/* 活跃会话徽章：多平台并列（取最新 lastSeenAt） */
function liveBadge(t) {
  const ss = t.sessions || [];
  if (!ss.length) return '';
  let latest = ss[0];
  for (const s of ss) {
    if ((Date.parse(s.lastSeenAt) || 0) > (Date.parse(latest.lastSeenAt) || 0)) latest = s;
  }
  const platforms = [...new Set(ss.map(s => s.platform))].join(' · ');
  // 超过 24h 的会话不再算"活跃"，徽章去光效淡化
  const stale = Date.now() - (Date.parse(latest.lastSeenAt) || 0) > 24 * 3600 * 1000;
  return `<span class="live${stale ? ' stale' : ''}"><i></i>${esc(platforms)} · ${esc(relTime(latest.lastSeenAt))}</span>`;
}

/* 观测台 LED 灯柱：3 档呼吸灯，按 displayState 决定点亮档位与节奏。
   档位映射：1=已就绪(ok) 2=进行中(work) 3=需要介入(alert/wait)。
   非纯颜色线索：亮起的档位 + 节奏差异，配合文字状态。 */
function observeLedFor(displayState) {
  let on = 1, beat = 0;
  const s = displayState || 'idle';
  if (s === 'blocked' || s === 'failed') { on = 3; beat = 1; }
  else if (s === 'waiting_permission' || s === 'waiting_question') { on = 3; beat = 0; }
  else if (s === 'working') { on = 2; beat = 1; }
  else if (s === 'turn_done' || s === 'completed') { on = 2; beat = 0; }
  else if (s === 'reviewing' || s === 'planning') { on = 1; beat = 0; }
  else { on = 1; beat = 0; }
  const cls = (i) => {
    const base = i <= on ? ' on' : '';
    const fast = beat && i === on ? ' beat-fast' : '';
    const slow = !fast && i === on && on > 1 ? ' beat' : '';
    return base + (fast || slow);
  };
  return `<span class="observe-led" aria-hidden="true"><i class="${cls(1)}"></i><i class="${cls(2)}"></i><i class="${cls(3)}"></i></span>`;
}

/* 观测台时间线：从真实时间戳派生 2-3 条「会话时间线」并横向压成一行，不伪造 activity。
   条目：会话开始(agent.startedAt) / 最近活动(agent.updatedAt) / 任务变更(lastChangedAt，缺失时用 mtime 兜底但文案保持「任务变更」)。
   缺失的时间戳整条省略；全部缺失返回低调空态。固定高度，不撑高卡片。 */
function observeTimelineFor(rt, t) {
  const entries = [];
  const sec = (v) => (typeof v === 'number' ? v : null);
  const push = (label, stamp) => {
    if (!stamp) return;
    entries.push(`<li><i aria-hidden="true"></i><span class="tl-label">${label}</span><span class="tl-time">${esc(relTime(stamp * 1000)) || '—'}</span></li>`);
  };
  const agent = rt && rt.agent;
  push('会话开始', sec(agent && agent.startedAt));
  push('最近活动', sec(agent && agent.updatedAt));
  push('任务变更', sec(rt && rt.lastChangedAt) ?? sec(t && t.mtime / 1000));
  if (!entries.length) {
    return `<div class="observe-timeline empty" aria-hidden="true"><span>暂无活动记录</span></div>`;
  }
  return `<ul class="observe-timeline" aria-label="最近活动">${entries.join('')}</ul>`;
}

/* ---------- 主卡片 ---------- */
function renderProjectActivityCard(activity) {
  const card = $('card'), main = $('main'), grid = $('grid');
  const displayState = displayStateForActivity(activity);
  /* Windows 路径（\\?\C:\... 或 C:\...）用 \ 分隔，统一按 / 和 \ 切取末段目录名 */
  const project = String(activity.project || '').replace(/[\\/]+$/, '').split(/[\\/]+/).filter(Boolean).pop() || activity.project;
  const agent = activity.agentKind || 'agent';
  const semanticActivity = displayActivity(
    activity.activity,
    activity.toolName,
    null,
    '会话正在运行',
    { eventName: activity.eventName, toolInput: activity.toolInput },
  );
  const rawActivity = String(activity.activity || '').trim();
  const updatedAt = (activity.updatedAt || 0) * 1000;
  card.dataset.kind = displayState === 'waiting_permission' || displayState === 'waiting_question' ? 'wrap' : 'work';
  card.dataset.runtimeState = displayState;
  for (const name of ['working', 'waiting-permission', 'waiting-question', 'blocked', 'failed', 'stale', 'done', 'completed']) {
    card.classList.toggle(`state-${name}`, displayState.replaceAll('_', '-') === name || (name === 'done' && displayState === 'turn_done'));
  }
  grid.classList.remove('center');
  renderPost(null);
  /* 项目级 fallback：与主任务卡同层级（title → timeline → hero），timeline 仅用真实 updatedAt 派生 */
  const rtLike = { agent: { startedAt: null, updatedAt: activity.updatedAt || 0 }, lastChangedAt: activity.updatedAt || 0 };
  main.innerHTML = `
    <div class="pane">
      <div class="head observe-head">
        <span class="obs-mark" aria-hidden="true"></span>
        <span class="repo">${esc(project)}</span>
        <span class="meter"><span class="ticks"><i class="c"></i></span><span class="score">AI</span></span>
      </div>
      <h2 class="title">${esc(agent)} 项目会话</h2>
      ${observeTimelineFor(rtLike, null)}
      <div class="runtime-hero state-${esc(displayState.replaceAll('_', '-'))}">
        ${observeLedFor(displayState)}
        <div class="observe-hero-inner">
        <div class="runtime-state-line"><span class="runtime-status-icon" aria-hidden="true">${icon(runtimeStateIcon(displayState), 15)}</span><strong id="runtime-state">${esc(semanticActivity || '会话正在运行')}</strong></div>
        <div id="runtime-activity" class="runtime-activity observe-stream">${runtimeActivityHtml(rawActivity || '最近没有 Agent 活动')}</div>
        </div>
      </div>
      <div class="runtime-row">
        <span class="runtime-phase">项目级 · 未绑定 Trellis task</span>
        <span class="runtime-progress">实时</span>
      </div>
      <div class="meta rule"><span class="chip br">${esc(project)}</span></div>
      <div class="stage"><div class="stage-fill">
        <div class="quote-label">会话来源</div>
        <div class="proj-context">
          <div class="pc-row"><span class="pc-key">Agent</span><span class="pc-val">${esc(agent)}</span></div>
          <div class="pc-row"><span class="pc-key">路径</span><span class="pc-val">${esc(project)}</span></div>
          <div class="pc-row"><span class="pc-key">最近活动</span><span class="pc-val">${esc(semanticActivity || '会话正在运行')}</span></div>
          <div class="pc-row"><span class="pc-key">更新</span><span class="pc-val">${esc(relTime(updatedAt)) || '—'}</span></div>
        </div>
      </div></div>
      <div class="foot"><span class="fid">${esc(activity.sessionId || 'session')}</span><span id="runtime-updated" class="when">更新于 ${esc(relTime(updatedAt)) || '—'}</span></div>
    </div>`;
}

function renderCard(t, projectActivity) {
  const card = $('card'), main = $('main'), grid = $('grid');
  card.dataset.runtimeState = 'idle';
  for (const name of ['working', 'waiting-permission', 'waiting-question', 'blocked', 'failed', 'stale', 'done', 'completed']) card.classList.remove(`state-${name}`);
  const fKey = t ? keyOf(t) : null;
  const changed = fKey !== lastFocusKey;
  if (changed) {
    /* 聚焦切换：重置回正面，文档缓存与标签选择失效 */
    state.flipped = false;
    state.prdCache = null;
    state.docSel = null;
    state.evidenceTarget = null;   /* 临时 evidence 展示随焦点切换清除 */
  }

  if (!state.projects.length && !projectActivity) {
    const waitingForHook = state.roots.length === 0;
    card.dataset.kind = 'idle';
    grid.classList.add('center');
    renderPost(null);
    main.innerHTML = waitingForHook
      ? `<div class="idle"><div class="w">等待项目接入</div><p>当前还没有项目<br/>打开设置安装 Hook，Agent 运行 Trellis 项目时会自动出现在这里</p></div>`
      : `<div class="idle"><div class="w">没有找到项目</div><p>当前根目录里没有 Trellis 项目<br/>打开设置调整扫描范围</p></div>`;
  } else if (t) {
    /* 任务候选优先于项目级会话：blocked/failed/waiting 等即使无 agent 也必须显示任务卡 */
    grid.classList.remove('center');
    const rt = runtimeViewForTask(t);
    const displayState = rt.displayState || 'idle';
    const runtimeKind = (displayState === 'blocked' || displayState === 'failed') ? 'halt' : displayState === 'completed' ? 'done' : displayState === 'reviewing' ? 'wrap' : displayState === 'planning' ? 'plan' : 'work';
    card.dataset.kind = KIND_COLOR[runtimeKind] ? runtimeKind : (KIND_COLOR[t.kind] ? t.kind : 'idle');
    card.dataset.runtimeState = displayState;
    for (const name of ['working', 'waiting-permission', 'waiting-question', 'blocked', 'failed', 'stale', 'done', 'completed']) {
      card.classList.toggle(`state-${name}`, displayState.replaceAll('_', '-') === name || (name === 'done' && displayState === 'turn_done'));
    }
    renderPost(t);
    const lane = Math.min(3, Math.max(0, t.lane ?? 0));
    const n = lane + 1;
    const ticks = LANES.map((_, i) => {
      const cls = (i < lane || t.kind === 'done') ? 'f' : (i === lane && t.kind !== 'done' ? 'c' : '');
      return `<i class="${cls}"></i>`;
    }).join('');
    const archiveReceipt = archiveReceiptFor(fKey);
    const actionLabel = rt.action ? (ACTION_COPY[rt.action] || rt.action) : '';
    const semanticActivity = archiveReceipt ? '已归档' : displayActivity(
      rt.activity,
      rt.agent && rt.agent.toolName,
      rt.action,
      actionLabel ? `${actionLabel} · 等待下一条事件` : '最近没有 Agent 活动',
      { eventName: rt.agent && rt.agent.eventName, toolInput: rt.agent && rt.agent.toolInput },
    );
    const rawActivity = (archiveReceipt && archiveReceipt.rawActivity) || String(rt.activity || '').trim();
    const runtimeNotice = archiveReceipt
      ? `<div class="runtime-unread archive-receipt" role="status"><span class="archive-receipt-copy">${icon('check', 11)}<span>任务已归档，当前任务已保留</span></span><button type="button" class="mini" data-next-after-archive>${icon('arrow-right', 10)}<span>查看下一个任务</span></button></div>`
      : (state.runtimeUnreadCount > 0 ? `<div class="runtime-unread">${state.runtimeNotice || '有未查看的实时活动'} · ${state.runtimeUnreadCount}<button type="button" class="mini" data-clear-unread>查看未读活动</button></div>` : '');
    const phaseLabel = t.phase && t.phase.label ? t.phase.label : (rt.phase || '—');
    const doneN = (t.subtasks || []).filter(s => s.status === 'completed' || s.status === 'done').length;
    const subLabel = (t.subtasks || []).length ? ` · 子任务 ${doneN}/${t.subtasks.length}` : '';
    const taskProgress = Math.round((t.progress || 0) * 100);
    const frontRefined = document.body.dataset.layout === 'front-refined';
    const progressLabel = frontRefined ? `开发 ${taskProgress}%` : `${taskProgress}%${esc(subLabel)}`;
    const priorityClass = t.priority ? ` priority-${String(t.priority).toLowerCase()}` : '';
    const updatedAt = rt.lastChangedAt ? rt.lastChangedAt * 1000 : t.mtime;
    main.innerHTML = `
      <div class="pane">
        <div class="head observe-head">
          <span class="obs-mark" aria-hidden="true"></span>
          <span class="repo">${esc(t.project)}</span>
          <span class="meter"><span class="ticks">${ticks}</span><span class="score">${n}<em>/4</em></span></span>
        </div>
        <h2 class="title">${esc(t.title || t.id)}</h2>
        ${observeTimelineFor(rt, t)}
        <div class="runtime-hero state-${esc(displayState.replaceAll('_', '-'))}">
          ${observeLedFor(displayState)}
          <div class="observe-hero-inner">
          <div class="runtime-state-line"><span class="runtime-status-icon" aria-hidden="true">${icon(runtimeStateIcon(displayState), 15)}</span><strong id="runtime-state">${esc(semanticActivity || DISPLAY_COPY[displayState] || displayState)}</strong></div>
        <div id="runtime-activity" class="runtime-activity observe-stream">${runtimeActivityHtml(rawActivity || (archiveReceipt ? '归档操作已完成' : '最近没有 Agent 活动'))}</div>
          ${runtimeNotice}
          ${`<button type="button" class="focus-control" data-focus-lock aria-pressed="${state.focusMode === 'manual'}">${focusLockHtml(state.focusMode === 'manual')}</button>`}
          </div>
        </div>
        <div class="runtime-row">
          <span id="runtime-phase" class="runtime-phase">Phase · ${esc(phaseLabel)}</span>
          <span id="runtime-progress" class="runtime-progress">${progressLabel}</span>
        </div>
        <div class="meta${t.artifacts ? '' : ' rule'}">
          ${t.priority ? `<span class="chip priority${priorityClass}">${esc(t.priority)}</span>` : ''}
          ${t.branch ? `<span class="chip br branch-meta">${esc(t.branch)}</span>` : ''}
          ${liveBadge(t)}
        </div>
        ${artsMini(t)}
        <div class="stage">${stageHtml(t, rt)}</div>
        <div class="foot">
          <span class="fid">${esc(t.id)}</span>
          <span id="runtime-updated" class="when">更新于 ${esc(relTime(updatedAt)) || '—'}</span>
        </div>
      </div>`;
    /* 子任务 bar：点击展开/收起清单（直接切 class 保住过渡动画） */
    const box = $('subsBox');
    if (box) {
      const toggle = () => {
        state.subOpen = state.subOpen === fKey ? null : fKey;
        box.classList.toggle('open', state.subOpen === fKey);
        box.querySelector('.hint').textContent = state.subOpen === fKey ? '收起' : '清单';
      };
      box.onclick = toggle;
      box.onkeydown = (e) => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggle(); }
      };
    }
    /* 锁定/解除锁定按钮：渲染后绑定（事件委托区域在 renderCard 内每次重建） */
    const lockBtn = main.querySelector('[data-focus-lock]');
    if (lockBtn) {
      lockBtn.onclick = () => {
        if (state.focusMode === 'manual') unlockFocus();
        else { state.focusMode = 'manual'; state.focusLockUntil = 0; render(); }
      };
      lockBtn.setAttribute('aria-pressed', String(state.focusMode === 'manual'));
      lockBtn.innerHTML = focusLockHtml(state.focusMode === 'manual');
    }
    /* 查看活动：切换到最新未读候选的正面活动卡；多个未读任务逐个查看。 */
    const unreadBtn = main.querySelector('[data-clear-unread]');
    if (unreadBtn) {
      unreadBtn.onclick = () => openUnreadActivity();
    }
    const archiveNextBtn = main.querySelector('[data-next-after-archive]');
    if (archiveNextBtn) {
      archiveNextBtn.onclick = () => continueAfterArchive();
    }
  } else if (projectActivity) {
    /* 仅在没有任务候选时展示项目级会话 */
    renderProjectActivityCard(projectActivity);
  } else {
    card.dataset.kind = 'idle';
    grid.classList.add('center');
    renderPost(null);
    main.innerHTML = `<div class="idle"><div class="w">空闲</div><p>没有进行中的任务<br/>开一个 Trellis task 再回来</p></div>`;
  }

  /* 动效：首次进入 stagger；聚焦切换时内容滑动淡入（GSAP 缺失 / 减少动态时跳过） */
  if (window.gsap && !reduced && state.mode === 'card') {
    const pane = main && main.querySelector('.pane');
    if (!entered) {
      entered = true;
      if (pane) gsap.from(pane.children, { y: 16, opacity: 0, duration: .55, stagger: .07, ease: 'power3.out', clearProps: 'all', delay: .06 });
    } else if (changed && pane) {
      gsap.fromTo(pane, { opacity: 0, x: 18 }, { opacity: 1, x: 0, duration: .38, ease: 'power3.out', clearProps: 'all' });
    }
  } else {
    entered = true;
  }
  applyFlipCard();
  lastFocusKey = fKey;
}

/* ---------- 翻面 ---------- */
/* 高度自适应只在正面生效；翻面必须移除 fit，否则 .face.back 被 display:none */
function syncFit() {
  /* fit 模式下 world 高度为 auto，让窗口跟随内容高度。
     设置面板（adminOpen）也参与自适应：内容高于当前窗口时拉伸窗口，
     而不是在固定窗口内内部滚动。翻面 / 任务树仍保持固定窗口几何。 */
  const fit = state.mode === 'card' && !state.flipped && !state.treeOpen;
  document.body.classList.toggle('fit', fit);
  /* fit 退出时清掉上一次写入的显式舞台高度，避免任务树 / 详情页继承正面卡片尺寸。 */
  if (!fit) {
    const world = $('world');
    if (world) {
      world.style.height = '';
      world.style.maxHeight = '';
    }
  }
  scheduleWindowFit();
}
/* 渲染稳定后一次性把窗口高度对齐到内容高度：防抖 + 阈值，避免连续 set_size 打死 WKWebView */
let fitWinTimer = null;
const IDLE_WINDOW_HEIGHT = 400;
function scheduleWindowFit() {
  if (state.mode !== 'card' || state.view !== 'main' || state.treeOpen) {
    clearTimeout(fitWinTimer);
    fitWinTimer = null;
    return;
  }
  clearTimeout(fitWinTimer);
  fitWinTimer = setTimeout(() => {
    /* 等 CSS 过渡（顶/底收起动画 .3s）落定后再量；用实际布局矩形而不是 scrollHeight（overflow:hidden 下不可靠） */
    const world = $('world'), card = $('card');
    if (!world || !card) return;
    /* 关键：测量时移除舞台当前的尺寸约束，取内容自然高度；测量完成后
       把目标高度显式写回舞台，避免原生窗口异步 resize 期间 flex 链又按旧视口收缩。 */
    world.style.height = 'auto';
    world.style.maxHeight = 'none';
    let h;
    if (state.flipped) {
      /* 背面是绝对定位、内部 .doc 自滚动，自然高度 = artsMini + dtabs 实高 + .doc.scrollHeight + 间距 */
      const back = $('back');
      if (!back) { world.style.maxHeight = ''; return; }
      const backTop = back.getBoundingClientRect().top - world.getBoundingClientRect().top;
      let content = 0, kids = 0;
      back.querySelectorAll('.b-pane > *').forEach(el => {
        kids++;
        content += el.classList.contains('doc') ? el.scrollHeight : el.getBoundingClientRect().height;
      });
      content += Math.max(0, kids - 1) * 8; /* .b-pane gap */
      /* 详情页限高：屏幕可用高度的 80% */
      h = Math.min(Math.ceil(backTop + content) + 4, Math.floor(screen.availHeight * 0.8));
    } else if (state.adminOpen) {
      /* 设置面板：窗口拉伸到覆盖 admin 全部内容，避免内部滚动截断。 */
      const admin = $('admin');
      const body = $('adminBody');
      if (!admin || !body) { world.style.maxHeight = ''; return; }
      const header = admin.querySelector('.list-h');
      const headerHeight = header ? header.getBoundingClientRect().height : 0;
      const panelHeight = Math.max(admin.getBoundingClientRect().height, headerHeight + body.scrollHeight);
      h = Math.min(
        Math.max(IDLE_WINDOW_HEIGHT, Math.ceil(panelHeight) + 4),
        Math.floor(screen.availHeight * 0.92),
      );
    } else {
      const bottom = card.getBoundingClientRect().bottom;
      h = Math.ceil(bottom - world.getBoundingClientRect().top) + 4; /* 舞台无 padding，只留边框取整余量 */
      /* 空闲页不随占位文案压缩：保持标准卡片窗口高度，菜单等绝对定位浮层
         才有稳定的可用空间。 */
      if (card.dataset.kind === 'idle') h = Math.max(h, IDLE_WINDOW_HEIGHT);
    }
    /* 原生窗口高度有同一套屏幕上限；舞台也同步使用该上限，避免原生层被
       截短时前端仍保留一个更高的内容层。 */
    if (Number.isFinite(screen.availHeight)) {
      h = Math.min(h, Math.max(160, Math.floor(screen.availHeight - 40)));
    }
    h = Math.max(160, Math.ceil(h));
    world.style.height = `${h}px`;
    world.style.maxHeight = `${h}px`;
    if (Math.abs(h - window.innerHeight) > 40) {
      call('fit_window_height', { height: h }).catch((error) => {
        console.error('[fit_window_height]', error);
      });
    }
  }, 500);
}
function applyFlipCard() {
  $('card').classList.toggle('flipped', state.flipped);
  $('btnFlip').innerHTML = `${icon(state.flipped ? 'chevron-left' : 'book-open', 13)}<span>${state.flipped ? '返回' : '详情'}</span>`;
  $('btnFlip').setAttribute('aria-label', state.flipped ? '返回任务观察' : '打开任务详情');
  syncFit();
}
function openUnreadActivity() {
  const action = TrellisFocusPolicy.resolveUnreadAction({
    mode: state.mode,
    treeOpen: state.treeOpen,
    adminOpen: state.adminOpen,
    hasCurrentFocus: Boolean(currentFocus()),
  });
  /* 面板打开或没有焦点时，不要让按钮变成“点击即丢失未读”。 */
  if (!action.shouldOpenActivityCard) {
    render();
    return false;
  }

  const evidence = latestUnreadEvidence();
  if (state.unreadEvidenceQueue.length && !evidence) {
    toast('未找到未读活动对应的任务，请刷新后重试');
    render();
    return false;
  }

  if (evidence) {
    /* 查看活动应进入对应任务的正面卡片，而不是当前任务的详情背面。 */
    state.archiveReceipt = null;
    state.focusKey = keyOf(evidence.task);
    state.flipped = false;
    state.evidenceTarget = null;
    state.prdCache = null;
    state.docSel = null;
    state.treeOpen = false;
    state.adminOpen = false;
    acknowledgeUnreadEvidence(evidence.entry);
  } else {
    /* 没有可定位的候选时，只确认当前入口，不伪装成跳到了活动任务。 */
    clearRuntimeUnread();
    state.flipped = false;
    state.evidenceTarget = null;
  }
  render();
  applyFlipCard();
  return true;
}
async function toggleFlip(force) {
  if (state.mode !== 'card' || state.treeOpen || state.adminOpen) return;
  const t = currentFocus();
  if (!t) return;
  const target = force !== undefined ? force : !state.flipped;
  if (target === state.flipped) return;
  state.flipped = target;
  if (!target) state.evidenceTarget = null;   /* 返回正面时清除临时 evidence 展示 */
  applyFlipCard();
  if (target) await loadBack(t);
}
/* 背面数据：artifacts 用 list_tasks 自带的，文档（prd/design/implement/调研/报告等）走 get_task（聚焦期内缓存）。
   activeTarget：本次展示的任务；异步返回时用它判断是否回填，避免 evidence 候选（非 currentFocus）停在加载中。 */
async function loadBack(t) {
  const key = keyOf(t);
  state.backActiveKey = key;
  if (state.prdCache && state.prdCache.key === key) {
    renderBack(t, state.prdCache.docs, false, state.prdCache.error);
    return;
  }
  renderBack(t, null, true);   /* 加载中 */
  try {
    const full = await call('get_task', { project: t.projectPath || t.project, id: t.id });
    state.prdCache = { key, docs: (full && full.docs) || [] };
  } catch (e) {
    console.error('[invoke:get_task]', e);
    state.prdCache = { key, docs: null, error: errMsg(e) };
  }
  /* 异步返回时若仍停在同一任务的背面（focus 或 evidence target）才填充 */
  const cur = currentFocus();
  const displayTarget = state.evidenceTarget || cur;
  if (TrellisFocusPolicy.shouldBackfillBack({
    flipped: state.flipped,
    activeKey: state.backActiveKey,
    targetKey: displayTarget && keyOf(displayTarget),
    curKey: key,
  })) {
    renderBack(t, state.prdCache.docs, false, state.prdCache.error);
  }
}
function renderBack(t, docs, loading, error) {
  const back = $('back');
  const rt = runtimeViewForTask(t);
  const phaseTrail = [t.phase && t.phase.id, rt.action, rt.displayState].filter(Boolean).map((item) => esc(DISPLAY_COPY[item] || ACTION_COPY[item] || item)).join(' → ');
  const runtimeTrail = rt.agent
    ? `<li><i></i>${esc(DISPLAY_COPY[rt.displayState] || rt.displayState)} · ${esc(rt.agent.toolName || rt.action || 'Agent')} · ${esc(relTime((rt.agent.updatedAt || 0) * 1000) || '刚刚')}</li>`
    : '<li><i></i>暂无实时 Agent 事件</li>';
  /* 文档标签：默认选中 PRD（或第一个），切换记忆在本次聚焦内 */
  let docsHtml = '';
  if (loading) {
    docsHtml = '<div class="doc dim">文档加载中…</div>';
  } else if (error) {
    docsHtml = [
      '<div class="doc dim">',
      '<p>文档读取失败：' + esc(error) + '</p>',
      '<button type="button" class="mini" data-retry-doc>重新加载</button>',
      '</div>',
    ].join('');
  } else if (!docs || !docs.length) {
    docsHtml = '<div class="doc dim">（这个任务还没有任何 markdown 文档）</div>';
  } else {
    const sel = docs.find(d => d.name === state.docSel) || docs[0];
    state.docSel = sel.name;
    const tabs = docs.map(d =>
      `<button type="button" class="dtab${d.name === sel.name ? ' on' : ''}" data-doc="${esc(d.name)}">${esc(d.label)}</button>`
    ).join('');
    docsHtml = `
      <div class="dtabs">${tabs}</div>
      <div class="doc">${mdRender(sel.content)}</div>`;
  }
  const detailMetrics = metricTags(t);
  const inspectMode = state.evidenceTarget && keyOf(state.evidenceTarget) === keyOf(t) ? 'evidence' : 'detail';
  const inspectHeader = inspectMode === 'evidence'
    ? `<div class="inspect-h"><span class="inspect-mode">活动证据</span><span class="inspect-task" title="${esc(t.title || t.id)}">${esc(t.title || t.id)}</span></div>`
    : '';
  const detailState = rt.displayState || 'idle';
  const detailPhase = t.phase && t.phase.label ? t.phase.label : (rt.phase || DISPLAY_COPY[detailState] || '—');
  const detailActivity = rt.activity || (rt.action ? `${ACTION_COPY[rt.action] || rt.action} · 等待下一条事件` : '最近没有 Agent 活动');
  const detailStatus = inspectMode === 'detail' ? `
      <div class="detail-status-line state-${esc(detailState.replaceAll('_', '-'))}" title="${esc(detailActivity)}">
        <span class="detail-status-icon" aria-hidden="true">${icon(runtimeStateIcon(detailState), 14)}</span>
        <strong>${esc(DISPLAY_COPY[detailState] || detailState)}</strong>
        <span class="detail-phase">Phase · ${esc(detailPhase)}</span>
        <span class="detail-activity">${esc(detailActivity)}</span>
      </div>` : '';
  const runtimeEvidence = inspectMode === 'evidence' ? `
      <div class="runtime-evidence">
        <div class="b-label">未读活动证据</div>
        <div class="phase-trail">${phaseTrail || '—'}</div>
        <div class="runtime-back-activity">${esc(rt.activity || '暂无活动摘要')}</div>
        <ul id="runtime-timeline" class="runtime-timeline">${runtimeTrail}</ul>
      </div>` : '';
  back.innerHTML = `
    <div class="b-pane">
      ${inspectHeader}
      ${detailStatus}
      ${artsMini(t)}
      ${detailMetrics ? `<div class="detail-metrics"><div class="b-label">任务指标</div>${detailMetrics}</div>` : ''}
      ${runtimeEvidence}
      ${docsHtml}
    </div>`;
  /* 标签切换：直接从缓存取，无网络往返 */
  back.querySelectorAll('.dtab').forEach(btn => {
    btn.onclick = () => {
      state.docSel = btn.dataset.doc;
      renderBack(t, docs, false, error);
    };
  });
  /* 重新加载：清除带 error 的缓存再取，保留任务上下文 */
  const retry = back.querySelector('[data-retry-doc]');
  if (retry) {
    retry.onclick = () => {
      state.prdCache = null;
      loadBack(t);
    };
  }
  scheduleWindowFit();   /* 文档高度变化（含异步加载完成、切标签）后重对齐窗口 */
}

/* ---------- markdown 渲染（vendor marked；walkTokens 转义原生 html；按 h2 重组折叠） ---------- */
const escHtml = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
/* 只注册一次：原生 HTML token 转义为文本（不引入 DOMPurify，源是本地可信文档） */
if (typeof marked !== 'undefined') {
  marked.use({
    walkTokens(token) {
      if (token.type === 'html') {
        token.type = 'text';
        token.text = escHtml(token.text);
      }
    },
  });
}
function mdRender(src) {
  const source = String(src || '');
  if (!window.marked) return escHtml(source); // vendor 未加载时的安全兜底
  const html = marked.parse(source);
  /* 按 <h2> 切分重组进折叠分区（替代原 parser 内嵌状态机）：
     h2 及后续内容进 <details class="doc-section">；h2 之前的头部内容原样保留。 */
  const h2Re = /<h2[^>]*>[\s\S]*?<\/h2>/g;
  const matches = html.match(h2Re) || [];
  if (!matches.length) return html;
  const out = [];
  let cursor = 0;
  for (const match of matches) {
    const idx = html.indexOf(match, cursor);
    if (idx > cursor) out.push(html.slice(cursor, idx));
    const title = match.replace(/^<h2[^>]*>/i, '').replace(/<\/h2>$/i, '');
    out.push(`<details class="doc-section" open><summary>${title}</summary><div class="doc-section-body">`);
    cursor = idx + match.length;
    /* 找下一个 h2 的位置作为本分区内容的结束 */
    const nextIdx = html.slice(cursor).search(h2Re);
    if (nextIdx >= 0) {
      out.push(html.slice(cursor, cursor + nextIdx));
      out.push('</div></details>');
      cursor += nextIdx;
    } else {
      out.push(html.slice(cursor));
      out.push('</div></details>');
      cursor = html.length;
    }
  }
  if (cursor < html.length) out.push(html.slice(cursor));
  return out.join('');
}

/* ---------- 层级树面板 ---------- */
function renderTree(focused) {
  $('list').classList.toggle('open', !!state.treeOpen);
  /* 同步「显示已归档」勾选框状态（加载持久化偏好或其它路径改动后保持一致） */
  const chkArchived = $('chkShowArchived');
  if (chkArchived) chkArchived.checked = !!state.showArchived;
  indexedTasks = [];
  const ul = $('tree');
  ul.setAttribute('role', 'tree');
  ul.setAttribute('aria-label', '任务树');
  ul.innerHTML = '';
  let total = 0;

  const names = state.filter ? [state.filter] : state.projects.map(p => p.name);
  for (const name of names) {
    const bucket = state.tasksByProject[name];
    const shown = trimCompleted(bucket ? bucket.tasks : [], state.showArchived);
    if (!shown.length) continue;

    const projLi = document.createElement('li');
    projLi.className = 'proj';
    const un = shown.filter(t => !t.archived && unfinished(t));
    let best = 'done';
    for (const t of un) {
      if ((KIND_URGENCY[t.kind] ?? 9) < (KIND_URGENCY[best] ?? 9)) best = t.kind;
    }
    const c = un.length ? (KIND_COLOR[best] || '#a6a7ae') : '#63656e';
    const head = document.createElement('div');
    head.className = 'proj-h';
    head.innerHTML = `<span class="d" style="background:${c};color:${c}"></span>${esc(name)}<span class="cnt">${shown.length}</span>`;
    projLi.appendChild(head);

    const kidsUl = document.createElement('ul');
    kidsUl.className = 'kids';
    const { top, kids } = buildTree(shown);
    /* 当前聚焦任务（或 runtime 候选）的父链自动展开，保证其可见 */
    const byId = new Map(shown.map(t => [t.id, t]));
    const focusTarget = (focused && byId.has(focused.id)) ? focused
      : (state.runtimeFocusKey && shown.find(t => keyOf(t) === state.runtimeFocusKey)) || null;
    if (focusTarget) {
      for (const pid of collectAncestors(focusTarget, byId)) treeCollapsed.delete(pid);
    }
    for (const task of top) kidsUl.appendChild(taskNode(task, kids, focused, 1));
    projLi.appendChild(kidsUl);
    ul.appendChild(projLi);
    total += shown.length;
  }

  if (!total) {
    ul.innerHTML = '<li class="tree-empty">暂无任务</li>';
  }
  $('listCount').textContent = `${total} 个任务`;
}
function taskNode(task, kids, focused, level) {
  const li = document.createElement('li');
  li.className = 'node';
  const key = keyOf(task);
  const isSelected = Boolean(focused && key === keyOf(focused));
  const sub = kids.get(task.id);
  const hasKids = !!(sub && sub.length);
  const collapsed = hasKids && treeCollapsed.has(key);
  const row = document.createElement('div');
  row.className = 'row' + (isSelected ? ' on' : '') + (task.archived ? ' archived' : '');
  row.tabIndex = 0;   /* 行本体可聚焦：Tab 可到达，Enter/Space 选中 */
  row.setAttribute('role', 'treeitem');
  row.setAttribute('aria-level', String(level || 1));
  row.setAttribute('aria-selected', String(isSelected));
  if (hasKids) {
    row.setAttribute('aria-expanded', String(!collapsed));
    row.setAttribute('aria-controls', 'kids-' + (key.replace(/[^A-Za-z0-9_-]/g, '-') || 'x'));
  }
  /* 当前 task 自身始终可见（含折叠父节点）→ 恒入索引、连续编号；
     折叠只排除 descendants，由下方 !collapsed 递归渲染控制。 */
  const num = indexedTasks.length < 9 ? String(indexedTasks.length + 1) : '';
  indexedTasks.push(task);
  const color = KIND_COLOR[task.kind] || '#a6a7ae';
  const pct = Math.round((typeof task.progress === 'number' ? task.progress : 0) * 100);
  /* 第二行：phase 或 runtime 状态（真实字段，不伪造）；lane 保留紧凑进度 */
  const rtView = runtimeViewForTask(task);
  const subLabel = (task.phase && task.phase.label)
    || (rtView && rtView.displayState && DISPLAY_COPY[rtView.displayState])
    || ((task.lane ?? 0) + 1 + '/4');
  const doneN = (task.subtasks || []).filter(s => s.status === 'completed' || s.status === 'done').length;
  const laneLabel = (task.subtasks || []).length ? `${doneN}/${task.subtasks.length}` : `${Math.round((task.progress || 0) * 100)}%`;
  row.innerHTML = `
    <span class="idx">${num}</span>
    ${hasKids ? `<button type="button" class="disclosure" data-key="${esc(key)}" tabindex="0"
      aria-label="${collapsed ? '展开' : '收起'} ${esc(task.title || task.id)}"
      aria-expanded="${!collapsed}" aria-controls="kids-${esc(key.replace(/[^A-Za-z0-9_-]/g, '-') || 'x')}">
      <span class="car" aria-hidden="true">${collapsed ? icon('chevron-right', 10) : icon('chevron-down', 10)}</span></button>` : '<span class="disclosure-slot" aria-hidden="true"></span>'}
    <span class="d" style="background:${color};color:${color}"></span>
    <div class="meta2">
      <div class="tline">
        <div class="t">${isSelected ? `<i class="sel-mark" aria-hidden="true">${icon('check', 9)}</i>` : ''}${esc(task.title || task.id)}</div>
        ${isLive(task) ? '<span class="live-dot" title="有活跃会话"></span>' : ''}
        <div class="bar"><i style="width:${pct}%;background:${color}"></i></div>
      </div>
      <div class="sub">${task.archived ? '<span class="archived-tag" title="已归档任务，仅查看">已归档</span>' : ''}${esc(subLabel)}</div>
    </div>
    ${task.archived ? '<span class="lane archived-lane" title="已归档">·</span>' : `<button type="button" class="tree-archive" data-key="${esc(key)}" title="归档 ${esc(task.title || task.id)}"
      aria-label="归档 ${esc(task.title || task.id)}">归档</button>`}
    <span class="lane" title="${esc(subLabel)}">${esc(laneLabel)}</span>`;
  const pick = () => focusTask(task);
  /* 点击行主体：选中（不切换展开） */
  row.onclick = (e) => {
    if (e.target.closest('.disclosure')) return;   /* 箭头只负责展开/收起 */
    if (e.target.closest('.tree-archive')) return; /* 归档按钮独立处理 */
    pick();
  };
  row.onkeydown = (e) => {
    if (e.target.closest('.disclosure')) return;
    if (e.target.closest('.tree-archive')) return;
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); pick(); }
  };
  li.appendChild(row);
  /* 归档按钮：hover 浮现；点击直接归档，不选中任务 */
  const archBtn = row.querySelector('.tree-archive');
  if (archBtn) {
    archBtn.onclick = (e) => {
      e.stopPropagation();
      archiveTreeTask(task, archBtn);
    };
    archBtn.onkeydown = (e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault(); e.stopPropagation();
        archiveTreeTask(task, archBtn);
      }
    };
  }
  /* disclosure 按钮：只切换收起状态，不选中、不关树 */
  const disc = row.querySelector('.disclosure');
  if (disc) {
    disc.onclick = (e) => {
      e.stopPropagation();
      toggleTreeCollapse(key);
    };
    disc.onkeydown = (e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault(); e.stopPropagation();
        toggleTreeCollapse(key);
      }
    };
  }
  if (hasKids && !collapsed) {
    const subUl = document.createElement('ul');
    subUl.className = 'kids';
    subUl.id = 'kids-' + (key.replace(/[^A-Za-z0-9_-]/g, '-') || 'x');
    for (const c of sub) subUl.appendChild(taskNode(c, kids, focused, (level || 1) + 1));
    li.appendChild(subUl);
  }
  return li;
}
/* 切换父节点收起状态；只影响树展示，不改 focusKey */
function toggleTreeCollapse(key) {
  if (treeCollapsed.has(key)) treeCollapsed.delete(key);
  else treeCollapsed.add(key);
  render();
}
/* 归档任务树中的任务：调用后端 task.py archive，成功后刷新列表。
   只对已完成任务展示按钮；调用后按钮防抖，避免重复触发。 */
async function archiveTreeTask(task, btn) {
  if (btn) {
    btn.disabled = true;
    btn.classList.add('busy');
  }
  try {
    await call('archive_task', { project: task.projectPath || task.project, task: task.dir || task.id });
    toast(`已归档：${task.title || task.id}`);
    await refresh(true);
    if (state.view === 'main') render();
  } catch (e) {
    toast(`归档失败：${errMsg(e)}`);
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.classList.remove('busy');
    }
  }
}
/* 返回当前任务（或 runtime 候选）到树根的父链；用于自动展开祖先。
   byId 是 id -> task 映射；沿显式 parent 字段向上，父不在可见列表则停。 */
function collectAncestors(task, byId) {
  const chain = [];
  const seen = new Set();
  let cur = task;
  while (cur && !seen.has(cur.id)) {
    seen.add(cur.id);
    if (!cur.parent || !byId.has(cur.parent)) break;
    chain.push(cur.parent);
    cur = byId.get(cur.parent);
  }
  return chain;
}
function focusTask(t) {
  state.archiveReceipt = null;
  state.focusKey = keyOf(t);
  state.focusMode = 'manual';
  state.focusLockUntil = 0;
  clearRuntimeUnread();
  state.treeOpen = false;
  state.subOpen = null;
  render();
}
/* 显式解除锁定：停止手动模式，回到策略决定焦点 */
function unlockFocus() {
  state.focusMode = 'auto';
  state.focusLockUntil = 0;
  clearRuntimeUnread();
  render();
}
/* 自动跟随开关：只处理开启之后的新事件，不回放旧事件。
   注意：开关与手动锁定独立——开启自动跟随绝不解除用户显式手动锁定；
   手动锁定只能由「解除锁定」退出（规格 4.3 / 验收 4）。 */
function toggleAutoFollow() {
  state.autoFollowImportant = !state.autoFollowImportant;
  state.autoFollowChangedAt = Date.now();
  if (!state.autoFollowImportant) {
    state.runtimeNotice = null;
  }
  /* 关闭开关时不清未读（保留 evidence 入口），开启时不触碰 focusMode/focusLockUntil */
  savePrefs();
  toast(state.autoFollowImportant ? '已开启自动跟随实时活动' : '已关闭自动跟随实时活动');
  render();
}

/* ---------- 胶囊：固定 360×136 三层信息 + 分区交互 ---------- */
let lastCapRuntimeState = null;
function capsuleKindFor(displayState, taskKind) {
  if (displayState === 'failed' || displayState === 'blocked') return 'halt';
  if (displayState === 'waiting_permission' || displayState === 'waiting_question') return 'wrap';
  if (displayState === 'completed' || displayState === 'turn_done') return 'done';
  if (displayState === 'reviewing') return 'wrap';
  if (displayState === 'planning') return 'plan';
  if (displayState === 'working') return 'work';
  return KIND_COLOR[taskKind] ? taskKind : 'idle';
}
function renderCapsule(t, projectActivity) {
  const cap = $('capsule');
  if (!cap) return;
  const show = state.mode === 'capsule';
  cap.hidden = !show;
  cap.classList.toggle('show', show);
  if (!show) {
    const pop = $('capMenuPop');
    if (pop) pop.hidden = true;
    const btn = $('btnCapMenu');
    if (btn) { btn.classList.remove('on'); btn.setAttribute('aria-expanded', 'false'); }
    return;
  }

  let displayState = 'idle';
  let title = '暂无活跃任务';
  let semanticActivity = '没有进行中的任务';
  let rawActivity = '';
  let progressText = '';
  let progressRatio = 0;
  let showBar = false;
  let kind = 'idle';
  let phaseText = '—';
  let projectName = '';
  let previewExcerpt = '';
  let progressTitle = '';

  if (t) {
    const rt = runtimeViewForTask(t);
    const archiveReceipt = archiveReceiptFor(keyOf(t));
    displayState = rt.displayState || 'idle';
    kind = capsuleKindFor(displayState, t.kind);
    title = t.title || t.id || '—';
    projectName = projectNameForPath(t.projectPath || t.project) || t.project;
    const actionLabel = rt.action ? (ACTION_COPY[rt.action] || rt.action) : '';
    const liveActivity = displayActivity(
      rt.activity,
      rt.agent && rt.agent.toolName,
      rt.action,
      '',
      { eventName: rt.agent && rt.agent.eventName, toolInput: rt.agent && rt.agent.toolInput },
    );
    semanticActivity = archiveReceipt ? '已归档' : (liveActivity || actionLabel || DISPLAY_COPY[displayState] || '最近没有 Agent 活动');
    rawActivity = (archiveReceipt && archiveReceipt.rawActivity) || String(rt.activity || '').trim();
    phaseText = t.phase && t.phase.label ? t.phase.label : (rt.phase || DISPLAY_COPY[displayState] || '—');
    previewExcerpt = t.excerpt || t.description || '';
    if (displayState === 'failed') semanticActivity = liveActivity || '执行失败，查看详情';
    if (displayState === 'blocked') semanticActivity = liveActivity || '任务已阻塞';
    if (displayState === 'waiting_permission') semanticActivity = liveActivity || '需要授权';
    if (displayState === 'waiting_question') semanticActivity = liveActivity || '等待你的回答';
    const subs = t.subtasks || [];
    if (subs.length) {
      const doneN = subs.filter(s => s.status === 'completed' || s.status === 'done').length;
      const currentSub = subs.find(s => s.status !== 'completed' && s.status !== 'done');
      const verification = /verify|验证|回归|test|测试|check|gate/i.test(currentSub && currentSub.name || '');
      const progressLabel = verification ? '验证' : '子任务';
      progressText = `${progressLabel} ${doneN}/${subs.length}`;
      progressTitle = `${verification ? '回归验证' : '子任务'}进度：${doneN}/${subs.length}`;
      progressRatio = doneN / subs.length;
    } else {
      const lane = Math.min(3, Math.max(0, t.lane ?? 0));
      progressText = `阶段 ${lane + 1}/4`;
      progressTitle = `生命周期阶段：${lane + 1}/4`;
      progressRatio = t.kind === 'done' || displayState === 'completed' ? 1 : (lane + (t.partial || 0)) / 4;
    }
    showBar = true;
  } else if (projectActivity) {
    displayState = displayStateForActivity(projectActivity);
    kind = capsuleKindFor(displayState, 'work');
    const project = String(projectActivity.project || '').replace(/[\\/]+$/, '').split(/[\\/]+/).filter(Boolean).pop() || projectActivity.project;
    projectName = projectNameForPath(projectActivity.project) || project;
    title = `${project} · ${projectActivity.agentKind || 'agent'}`;
    semanticActivity = displayActivity(
      projectActivity.activity,
      projectActivity.toolName,
      null,
      '项目级会话运行中',
      { eventName: projectActivity.eventName, toolInput: projectActivity.toolInput },
    );
    rawActivity = String(projectActivity.activity || '').trim();
    phaseText = '项目级会话';
    previewExcerpt = semanticActivity;
    progressText = '';
    showBar = false;
    /* 项目级会话：状态行显示「项目会话」，不伪造任务进度 */
    $('capState').textContent = `项目会话 · ${semanticActivity || DISPLAY_COPY[displayState] || '实时'}`;
  } else {
    displayState = 'idle';
    kind = 'idle';
    title = '暂无活跃任务';
    semanticActivity = '开一个 Trellis task 再回来';
    phaseText = '—';
    previewExcerpt = '当前没有活跃任务';
    progressText = '';
    showBar = false;
  }

  cap.dataset.kind = kind;
  cap.dataset.runtimeState = displayState;
  const orb = $('capOrb');
  const completed = displayState === 'completed' || displayState === 'turn_done';
  if (orb) {
    orb.classList.toggle('check', completed);
    orb.innerHTML = completed ? icon('check', 9) : '';
    orb.title = DISPLAY_COPY[displayState] || displayState || '空闲';
  }
  if (!(projectActivity && !t)) {
    $('capState').textContent = semanticActivity || DISPLAY_COPY[displayState] || displayState || '空闲';
  }
  $('capTitle').textContent = title;
  $('capTitle').title = title;
  const capsuleActivity = rawActivity || semanticActivity;
  $('capActivity').textContent = capsuleActivity;
  /* 项目标识：始终展示当前所在项目，空闲态显示全部/未接入 */
  const capProject = $('capProject');
  const previewProject = $('capPreviewProject');
  const projectText = projectName || (state.filter || '全部项目');
  if (capProject) {
    capProject.textContent = projectText;
    capProject.title = `当前项目 · ${projectText}`;
    capProject.hidden = !projectText;
  }
  if (previewProject) previewProject.textContent = projectText;
  const prog = $('capProgress');
  if (progressText) {
    prog.hidden = false;
    prog.textContent = progressText;
    prog.title = progressTitle;
  } else {
    prog.hidden = true;
    prog.textContent = '';
    prog.title = '';
  }
  const bar = $('capBar');
  const fill = $('capBarFill');
  if (showBar) {
    bar.hidden = false;
    fill.style.width = `${Math.round(Math.max(0, Math.min(1, progressRatio)) * 100)}%`;
  } else {
    bar.hidden = true;
    fill.style.width = '0%';
  }

  /* 胶囊默认只显示三行；内容区 hover 时复用同一份真实运行数据展开只读预览。 */
  const previewState = $('capPreviewState');
  const previewPhase = $('capPreviewPhase');
  const previewProgress = $('capPreviewProgress');
  const previewTitle = $('capPreviewTitle');
  const previewActivity = $('capPreviewActivity');
  const previewExcerptEl = $('capPreviewExcerpt');
  if (previewState) previewState.textContent = semanticActivity || DISPLAY_COPY[displayState] || displayState || '空闲';
  if (previewPhase) previewPhase.textContent = `Phase · ${phaseText}`;
  if (previewProgress) {
    previewProgress.textContent = t && progressText
      ? `开发 ${Math.round(Math.max(0, Math.min(1, t.progress || 0)) * 100)}% · ${progressText}`
      : (progressText ? `进度 ${progressText}` : '实时');
  }
  if (previewTitle) previewTitle.textContent = title;
  if (previewActivity) previewActivity.textContent = capsuleActivity;
  if (previewExcerptEl) previewExcerptEl.textContent = previewExcerpt || capsuleActivity;

  /* 未读 badge：manual lock / 关闭自动跟随时的被拒实时活动 */
  const badge = $('capBadge');
  const unread = state.runtimeUnreadCount || 0;
  if (unread > 0) {
    badge.hidden = false;
    badge.textContent = unread > 99 ? '99+' : String(unread);
    badge.title = state.runtimeNotice || '有新的实时活动';
  } else {
    badge.hidden = true;
    badge.textContent = '0';
  }

  /* 异常态一次 pulse（同状态不重复；reduced-motion 由 CSS 关掉） */
  const abnormal = ['waiting_permission', 'waiting_question', 'blocked', 'failed'].includes(displayState);
  if (abnormal && displayState !== lastCapRuntimeState && !reduced) {
    cap.classList.remove('pulse');
    void cap.offsetWidth;
    cap.classList.add('pulse');
    clearTimeout(renderCapsule._pulseT);
    renderCapsule._pulseT = setTimeout(() => cap.classList.remove('pulse'), 600);
  }
  lastCapRuntimeState = displayState;

  /* 同步胶囊菜单开关态 */
  syncCapMenu();
}
function syncCapMenu() {
  const lock = $('btnCapLock');
  if (lock) {
    const manual = state.focusMode === 'manual';
    lock.setAttribute('aria-pressed', String(manual));
    lock.textContent = manual ? '解除锁定' : '锁定当前任务';
  }
  const auto = $('btnCapAutoFollow');
  if (auto) {
    auto.setAttribute('aria-checked', String(!!state.autoFollowImportant));
    auto.classList.toggle('on', !!state.autoFollowImportant);
    const mark = auto.querySelector('i');
    if (mark) mark.textContent = state.autoFollowImportant ? '开' : '关';
  }
  const top = $('btnCapTop');
  if (top) {
    top.setAttribute('aria-checked', String(!!state.alwaysOnTop));
    top.classList.toggle('on', !!state.alwaysOnTop);
    const mark = top.querySelector('i');
    if (mark) mark.textContent = state.alwaysOnTop ? '开' : '关';
  }
}
function toggleCapMenu(force) {
  const pop = $('capMenuPop');
  const btn = $('btnCapMenu');
  if (!pop || !btn) return;
  const target = force !== undefined ? force : pop.hidden;
  pop.hidden = !target;
  btn.classList.toggle('on', target);
  btn.setAttribute('aria-expanded', String(target));
  if (target) {
    /* 与主卡菜单一致：记录触发按钮，焦点进入第一个菜单项 */
    focusReturnTo = btn;
    syncCapMenu();
    setTimeout(() => moveFocusInto(pop), 50);
  } else {
    restoreFocus();
  }
}
async function openUnreadFromCapsule() {
  /* badge：回卡片查看证据，不改 focus/锁定。切换失败时保留未读。 */
  if (state.mode === 'capsule') {
    try {
      await call('set_window_mode', { mode: 'card' });
    } catch (e) {
      report('set_window_mode', e);
      return false;
    }
    state.mode = 'card';
    render();
  }
  return openUnreadActivity();
}
async function setMode(mode) {
  if (state.mode === mode) return;
  try {
    await call('set_window_mode', { mode });
    state.mode = mode;
    if (mode === 'capsule') {
      state.themeOpen = false;
      state.menuOpen = false;
      const mp = $('menuPop');
      if (mp) mp.hidden = true;
      toggleCapMenu(false);
    } else {
      toggleCapMenu(false);
    }
    render();
  } catch (e) {
    report('set_window_mode', e);
  }
}

/* ---------- 管理面板 ---------- */
function hookAgent(agent) {
  return HOOK_AGENTS.find((item) => item.id === agent) || HOOK_AGENTS[0];
}
function hookStatus(agent) {
  return state.hookStatuses.find((item) => item && item.agent === agent) || null;
}
function hookStatusLabel(agent) {
  if (state.hookUpdatingAgent === agent) return '处理中…';
  if (state.hookStatusLoading) return '检查中…';
  if (state.hookStatusError) return '读取失败';
  const status = hookStatus(agent);
  return status ? (status.installed ? '已安装' : '未安装') : '未检测';
}
function hookStatusTone(agent) {
  if (state.hookUpdatingAgent === agent) return 'checking';
  if (state.hookStatusLoading) return 'checking';
  if (state.hookStatusError) return 'error';
  const status = hookStatus(agent);
  return status && status.installed ? 'installed' : 'missing';
}
function renderHookSection(body) {
  body.appendChild(secTitle('Agent 接入'));

  const guide = document.createElement('div');
  guide.className = 'hook-guide';
  guide.innerHTML = `<strong>让 Trellis Card 自动收到活动</strong><span>本机全局接入，一次安装即可观察多个 Trellis 项目；普通项目不会进入卡片。</span><span>选择 Agent，点击安装，然后重启对应 Agent。移除时只会删除 Trellis Card 自己的 Hook。</span>`;
  body.appendChild(guide);

  const actions = document.createElement('div');
  actions.className = 'hook-actions';
  const refreshButton = document.createElement('button');
  refreshButton.type = 'button';
  refreshButton.className = 'mini hook-refresh';
  refreshButton.innerHTML = `${icon('refresh', 12)}<span>重新检测</span>`;
  refreshButton.disabled = state.hookStatusLoading || !!state.hookUpdatingAgent;
  refreshButton.onclick = () => refreshHookStatuses(true);
  actions.appendChild(refreshButton);
  body.appendChild(actions);

  for (const agent of HOOK_AGENTS) {
    const status = hookStatus(agent.id);
    const row = document.createElement('div');
    row.className = 'adm-row hook-row';
    row.dataset.hookAgent = agent.id;
    const tone = hookStatusTone(agent.id);
    const stateLabel = hookStatusLabel(agent.id);
    const statusIcon = tone === 'installed' ? 'check' : tone === 'error' ? 'circle-x' : tone === 'checking' ? 'refresh' : 'circle-alert';
    const configPath = (status && status.configPath) || agent.configPath;
    row.innerHTML = `
      <div class="adm-info">
        <div class="adm-name">${esc(agent.label)}<span class="hook-state hook-state-${tone}">${icon(statusIcon, 11)}<span>${esc(stateLabel)}</span></span></div>
        <div class="hook-desc">${esc(agent.description)}</div>
        <div class="adm-path">配置：${esc(configPath)}</div>
      </div>`;
    const button = document.createElement('button');
    button.type = 'button';
    button.className = status && status.installed ? 'mini danger-mini hook-action' : 'mini hook-action';
    button.disabled = !status || !!state.hookStatusError || !!state.hookStatusLoading || !!state.hookUpdatingAgent;
    if (state.hookUpdatingAgent === agent.id) {
      button.innerHTML = `${icon('refresh', 11)}<span>处理中…</span>`;
    } else {
      button.innerHTML = status && status.installed
        ? `${icon('x', 11)}<span>移除</span>`
        : `${icon('check', 11)}<span>安装</span>`;
    }
    button.setAttribute('aria-label', `${agent.label}${status && status.installed ? '移除' : '安装'} Hook`);
    button.onclick = () => toggleHook(agent.id);
    row.appendChild(button);
    body.appendChild(row);
  }

  if (state.hookStatusError) {
    const error = note(`读取 Hook 配置失败：${state.hookStatusError}`);
    error.classList.add('hook-error');
    body.appendChild(error);
  }
}
async function refreshHookStatuses(force = false) {
  if (state.hookStatusLoading || (state.hookStatusRequested && !force)) return;
  state.hookStatusRequested = true;
  state.hookStatusLoading = true;
  state.hookStatusError = null;
  if (state.adminOpen) render();
  try {
    const statuses = await call('get_hook_statuses');
    if (!Array.isArray(statuses)) throw new Error('返回的 Hook 状态格式无效');
    state.hookStatuses = statuses.filter((item) => item && HOOK_AGENTS.some((agent) => agent.id === item.agent));
  } catch (e) {
    state.hookStatusError = errMsg(e);
    console.error('[invoke:get_hook_statuses]', e);
  } finally {
    state.hookStatusLoading = false;
    if (state.adminOpen) render();
  }
}
async function toggleHook(agent) {
  const current = hookStatus(agent);
  if (!current || state.hookStatusLoading || state.hookUpdatingAgent) return;
  const enabled = !current.installed;
  const label = hookAgent(agent).label;
  state.hookUpdatingAgent = agent;
  state.hookStatusError = null;
  render();
  try {
    const next = await call('configure_hook', { agent, enabled });
    if (!next || next.agent !== agent) throw new Error('返回的 Hook 状态格式无效');
    state.hookStatuses = state.hookStatuses.map((item) => item.agent === agent ? next : item);
    toast(enabled ? `${label} Hook 已安装，请重启 ${label}` : `${label} Hook 已移除`);
  } catch (e) {
    console.error('[invoke:configure_hook]', e);
    toast(`${label} Hook 操作失败：${errMsg(e)}`);
  } finally {
    state.hookUpdatingAgent = null;
    render();
  }
}
function renderAdmin() {
  const panel = $('admin');
  document.body.classList.toggle('admin-open', !!state.adminOpen);
  panel.classList.toggle('open', !!state.adminOpen);
  if (!state.adminOpen) {
    scheduleWindowFit();
    return;
  }
  if (!state.hookStatusRequested && !state.hookStatusLoading) refreshHookStatuses();
  const body = $('adminBody');
  body.innerHTML = '';

  renderHookSection(body);

  /* Configure：根目录可写在前，已发现项目只读在后 */
  body.appendChild(secTitle('根目录'));
  if (!state.roots.length) body.appendChild(note('还没有根目录'));
  for (const r of state.roots) {
    const row = document.createElement('div');
    row.className = 'adm-row';
    row.innerHTML = `<div class="adm-info"><div class="adm-path" style="font-size:9.5px;color:var(--mist)">${esc(r)}</div></div>`;
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'mini danger-mini';
    btn.textContent = '移除';
    btn.onclick = () => removeRoot(r);
    row.appendChild(btn);
    body.appendChild(row);
  }
  const add = document.createElement('button');
  add.type = 'button';
  add.className = 'mini block';
  add.textContent = '＋ 添加文件夹';
  add.onclick = () => addRootFlow(false);
  body.appendChild(add);

  body.appendChild(secTitle('已发现项目'));
  if (!state.projects.length) body.appendChild(note('没有发现 Trellis 项目'));
  for (const p of state.projects) {
    const row = document.createElement('div');
    row.className = 'adm-row';
    row.innerHTML = `
      <div class="adm-info">
        <div class="adm-name">${esc(p.name)}<span class="adm-count">${p.taskCount ?? 0} 任务</span></div>
        <div class="adm-path">${esc(projectNameForPath(p.path) || p.path)}</div>
      </div>`;
    body.appendChild(row);
  }
  setTimeout(scheduleWindowFit, 650);
}
function secTitle(text) {
  const d = document.createElement('div');
  d.className = 'adm-sec';
  d.textContent = text;
  return d;
}
function note(text) {
  const d = document.createElement('div');
  d.className = 'adm-note';
  d.textContent = text;
  return d;
}
async function removeRoot(path) {
  try {
    const r = await call('remove_root', { path });
    state.roots = (r && r.roots) || [];
    toast('已移除根目录');
    await refresh(true);
    render();
  } catch (e) {
    report('remove_root', e);
  }
}
/* fromSetup=true 时添加成功直接进入主界面 */
async function addRootFlow(fromSetup) {
  if (fromSetup && state.setupBusy) return;
  if (fromSetup) {
    state.setupBusy = true;
    renderSetup();
  }
  try {
    const path = await call('pick_folder');
    if (!path) return;   /* 用户取消 */
    const r = await call('add_root', { path });
    state.roots = (r && r.roots) || [];
    toast('已添加文件夹');
    if (fromSetup) {
      await enterMain({ openHookSetup: true });
    } else {
      await refresh(true);
      render();
    }
  } catch (e) {
    report('add_root', e);
  } finally {
    if (fromSetup && state.view === 'setup') {
      state.setupBusy = false;
      renderSetup();
    }
  }
}

/* ---------- 工具条 ---------- */
function syncTools() {
  /* 统一走菜单 chrome + 胶囊菜单态 */
  syncMenuChrome();
  syncCapMenu();
}
async function toggleTop() {
  const flag = !state.alwaysOnTop;
  try {
    const r = await call('set_always_on_top', { flag });
    state.alwaysOnTop = !!r;
    toast(state.alwaysOnTop ? '已置顶' : '已取消置顶');
    render();
  } catch (e) {
    report('set_always_on_top', e);
  }
}
async function hideWindow() {
  try {
    await call('hide_window');
  } catch (e) {
    report('hide_window', e);
  }
}
/* 焦点管理：把焦点移入面板第一个可操作控件（或面板本身），关闭后恢复触发按钮 */
let focusReturnTo = null;   /* 打开 sheet/菜单前记住触发按钮，关闭时恢复 */
function moveFocusInto(panel) {
  if (!panel) return;
  const first = panel.querySelector('button, a, input, [tabindex]:not([tabindex="-1"])');
  if (first) first.focus();
  else if (panel.setAttribute) { panel.setAttribute('tabindex', '-1'); panel.focus(); }
}
function restoreFocus() {
  if (focusReturnTo && document.contains(focusReturnTo)) focusReturnTo.focus();
  focusReturnTo = null;
}
function toggleTree() {
  const opening = !state.treeOpen;
  if (opening) {
    if (state.themeOpen) toggleThemePop(false);
    if (state.menuOpen) toggleMenu(false);
    closeProjectPop();
    state.adminOpen = false;
    state.flipped = false;   /* 开 sheet 必回正面 */
    state.evidenceTarget = null;
    state.treeOpen = true;
    focusReturnTo = $('btnTree');
    render();
    setTimeout(() => moveFocusInto($('list')), 50);
  } else {
    state.treeOpen = false;
    render();
    restoreFocus();
  }
}
function toggleAdmin() {
  const opening = !state.adminOpen;
  if (opening) {
    if (state.themeOpen) toggleThemePop(false);
    if (state.menuOpen) toggleMenu(false);
    closeProjectPop();
    state.treeOpen = false;
    state.flipped = false;
    state.evidenceTarget = null;
    state.adminOpen = true;
    focusReturnTo = document.activeElement && document.activeElement.id === 'btnAdmin'
      ? $('btnAdmin')
      : ($('btnMenu') || $('btnAdmin'));
    render();
    setTimeout(() => moveFocusInto($('admin')), 50);
  } else {
    state.adminOpen = false;
    render();
    restoreFocus();
  }
}
function closeTree() {
  if (!state.treeOpen) return;
  state.treeOpen = false;
  render();
  restoreFocus();
}
function closeAdmin() {
  if (!state.adminOpen) return;
  state.adminOpen = false;
  render();
  restoreFocus();
}

/* ---------- 设置引导 ---------- */
function renderSetup() {
  state.view = 'setup';
  $('setup').hidden = false;
  $('mainView').hidden = true;
  for (const id of ['btnScanExisting', 'btnStartEmpty']) {
    const button = $(id);
    if (button) button.disabled = state.setupBusy;
  }
}
async function startWithoutProject() {
  if (state.setupBusy) return;
  state.setupBusy = true;
  renderSetup();
  try {
    await call('complete_setup');
    await enterMain({ openHookSetup: true });
  } catch (e) {
    report('complete_setup', e);
  } finally {
    if (state.view === 'setup') {
      state.setupBusy = false;
      renderSetup();
    }
  }
}
async function enterMain({ openHookSetup = false } = {}) {
  state.view = 'main';
  state.configured = true;
  $('setup').hidden = true;
  $('mainView').hidden = false;
  await refresh(true);
  render();
  if (openHookSetup && !state.adminOpen) toggleAdmin();
}

/* ---------- 定时器 ---------- */
async function pollTasks() {
  if (state.view !== 'main' || document.hidden) return;
  await refresh(false);
}

/* ---------- 后端事件：文件监听推送 tasks-changed ---------- */
let watchLastAt = 0;   /* 1s 节流防连发 */
let hookRefreshTimer = null;
let hookRefreshRetryTimer = null;
let hookRefreshRetryPayload = null;
let hookRefreshPayload = null;
let hookRefreshChain = Promise.resolve();

/* Hook 事件可能在一次 Agent 调用内连续到达。任务列表刷新是全项目扫描，
   只合并明确的列表变化信号，并串行执行，避免重复扫描和并发覆盖状态。 */
function mergeHookRefreshPayload(current, next) {
  if (!current) return { ...next, projectConflict: false };
  const projectConflict = current.projectConflict
    || Boolean(current.project && next.project && current.project !== next.project);
  const action = current.action === 'create' || next.action === 'create'
    ? 'create'
    : (next.action || current.action || null);
  return {
    dynamicProjectAdded: Boolean(current.dynamicProjectAdded || next.dynamicProjectAdded),
    action,
    project: projectConflict ? null : (current.project || next.project || null),
    projectConflict,
  };
}

function queueHookRefresh(payload, delay = 250) {
  hookRefreshPayload = mergeHookRefreshPayload(hookRefreshPayload, payload || {});
  if (hookRefreshTimer !== null) return;
  hookRefreshTimer = setTimeout(() => {
    hookRefreshTimer = null;
    const pending = hookRefreshPayload;
    hookRefreshPayload = null;
    if (!pending) return;
    hookRefreshChain = hookRefreshChain
      .then(() => onTasksChanged({ urgent: true, project: pending.project }))
      .catch(e => console.error('[event:hook-tasks-changed]', e));
  }, delay);
}

function queueHookRefreshRetry(payload) {
  hookRefreshRetryPayload = mergeHookRefreshPayload(hookRefreshRetryPayload, payload || {});
  if (hookRefreshRetryTimer !== null) return;
  hookRefreshRetryTimer = setTimeout(() => {
    hookRefreshRetryTimer = null;
    const pending = hookRefreshRetryPayload;
    hookRefreshRetryPayload = null;
    if (pending) queueHookRefresh(pending, 0);
  }, 700);
}

async function onTasksChanged({ urgent = false, project = null } = {}) {
  const now = Date.now();
  if (!urgent && now - watchLastAt < 1000) return;
  watchLastAt = now;
  if (state.view !== 'main') {
    try {
      const projects = await call('list_projects');
      if (projects && projects.length) await enterMain();
    } catch (e) {
      report('list_projects', e);
    }
    return;
  }
  const projectName = projectNameForPath(project);
  const shouldRevealProject = Boolean(
    projectName
      && state.filter
      && state.filter !== projectName
      && state.focusMode !== 'manual',
  );
  if (shouldRevealProject) {
    /* Trellis actions are explicit activity for the project being planned or
       created. Do not let a stale project filter hide that activity. */
    state.filter = null;
    state.focusKey = null;
    state.runtimeFocusKey = null;
  }
  const before = new Set(pool().map(keyOf));
  await refresh(true);
  if (shouldRevealProject) {
    /* Refresh has repopulated all projects. Keep the newly revealed project
       visible even when the runtime focus snapshot still points at an older
       task from before the hook arrived. */
    state.runtimeFocusKey = null;
    const bucket = state.tasksByProject[projectName];
    const target = (bucket && bucket.tasks || [])
      .filter(unfinished)
      .sort((a, b) => (b.mtime || 0) - (a.mtime || 0))[0];
    if (target) {
      state.focusKey = keyOf(target);
      state.flipped = false;
      state.prdCache = null;
      state.docSel = null;
    }
  }
  /* 对比刷新前后任务池，聚焦并提示新任务 */
  for (const t of pool()) {
    if (before.has(keyOf(t))) continue;
    const newTaskKey = keyOf(t);
    if (TrellisFocusPolicy.shouldFocusNewTaskOnCreate({
      enabled: state.autoFollowImportant,
      locked: state.focusMode === 'manual',
      runtimeFocusKey: state.runtimeFocusKey,
      newTaskKey,
    })) {
      state.focusKey = newTaskKey;
      state.flipped = false;
      state.prdCache = null;
      state.docSel = null;
    }
    state.subOpen = null;
    toast(`新任务：${t.title || t.id}`);
    if (state.mode === 'capsule') await setMode('card');   /* 胶囊模式自动弹回 */
  }
  render();
}
function bindWatch() {
  if (!hasTauri || !window.__TAURI__.event) return;
  window.__TAURI__.event.listen('tasks-changed', () => {
    onTasksChanged().catch(e => console.error('[event:tasks-changed]', e));
  });
  window.__TAURI__.event.listen('runtime-reconciliation-needed', () => {
    refresh(true).catch(e => console.error('[event:runtime-reconciliation-needed]', e));
  });
  window.__TAURI__.event.listen('hook-tasks-changed', (event) => {
    const payload = event && event.payload ? event.payload : {};
    queueHookRefresh(payload);
    /* task.py create 的 Hook 常发生在 task.json 写入前，延迟复查一次。 */
    if (payload.action === 'create') {
      queueHookRefreshRetry(payload);
    }
  });
  window.__TAURI__.event.listen('agent-state-changed', (event) => {
    applyRuntimeSnapshot(event && event.payload ? event.payload : {});
    if (state.view === 'main') render();
  });
  /* 完成迁移事件（后端 canonical 检测）：幂等消费——只触发一次刷新，
     完成态推进由 applyRuntimeSnapshot 的 focus-policy 层处理，不重复未读/推进。 */
  window.__TAURI__.event.listen('task-completed', () => {
    refresh(true).catch(e => console.error('[event:task-completed]', e));
  });
  window.__TAURI__.event.listen('focus-task-changed', (event) => {
    const key = event && event.payload;
    if (typeof key === 'string') {
      const [project, ...id] = key.split('::');
      const projectInfo = state.projects.find(p => p.path === project || p.name === project);
      const nextRuntimeFocusKey = projectInfo ? `${projectInfo.name}::${id.join('::')}` : key;
      state.runtimeFocusKey = nextRuntimeFocusKey;
      const currentRuntimeView = state.focusKey ? state.runtimeByTask.get(state.focusKey) : null;
      /* 后端可能先发 focus-task-changed，再发完成快照；归档动作必须先生成回执，
         否则这里会在 applyRuntimeSnapshot 之前把当前任务切走。 */
      if (state.focusKey
        && nextRuntimeFocusKey !== state.focusKey
        && currentRuntimeView
        && currentRuntimeView.action === 'archive') {
        holdArchivedFocus(state.focusKey, currentRuntimeView);
      }
      /* 采纳策略：只更新焦点，不记未读。Rust 的 emit_runtime_snapshot 先发本事件、
         再发 agent-state-changed（含完整 snapshot），未读统一由 applyRuntimeSnapshot
         按最新 lastChangedAt 去重记录，避免双路径重复计数。
         用 classifyRuntimeCandidate 统一判断：refresh-same 时不动（原地刷新由 render 表达）。 */
      const focusView = state.runtimeByTask.get(nextRuntimeFocusKey);
      const focusClass = TrellisFocusPolicy.classifyRuntimeCandidate({
        enabled: state.archiveReceipt ? false : state.autoFollowImportant,
        locked: state.focusMode === 'manual',
        hasCurrentFocus: Boolean(state.focusKey),
        currentKey: state.focusKey,
        candidateKey: nextRuntimeFocusKey,
        nextState: focusView && focusView.displayState,
        filter: state.filter,
        nextProject: runtimeProjectName(focusView),
        isNewSinceEnabled: isNewSinceAutoFollowChanged(focusView),
      });
      if (focusClass === 'adopt' && state.focusKey !== nextRuntimeFocusKey) {
        state.focusKey = nextRuntimeFocusKey;
        state.flipped = false;
        state.prdCache = null;
        state.docSel = null;
        clearRuntimeUnread();
        /* 已采纳候选不再被去重标记占用 */
        state.lastUnreadCandidateKey = null;
        state.lastUnreadCandidateStamp = 0;
      }
      if (state.view === 'main') render();
    }
  });
}

/* ---------- 主卡观察菜单 ---------- */
function syncThemeChrome() {
  const btn = $('btnTheme');
  const pop = $('themePop');
  const name = $('themeName');
  if (btn) {
    btn.classList.toggle('on', !!state.themeOpen);
    btn.setAttribute('aria-expanded', String(!!state.themeOpen));
  }
  if (pop) pop.hidden = !state.themeOpen;
  if (name) name.textContent = themeLabel(state.theme);
  document.querySelectorAll('[data-theme-choice]').forEach((choice) => {
    choice.setAttribute('aria-pressed', String(choice.dataset.themeChoice === state.theme));
  });
}
function renderThemePicker() {
  const root = $('themeChoices');
  if (!root || root.childElementCount) {
    syncThemeChrome();
    return;
  }
  root.innerHTML = THEME_DEFS.map((theme) => `
    <button type="button" class="theme-choice" data-theme-choice="${theme.id}" aria-pressed="false" title="${esc(theme.family)}">
      <span class="theme-swatch" style="--swatch-a:${theme.swatch[0]};--swatch-b:${theme.swatch[1]}" aria-hidden="true"></span>
      <span class="theme-choice-label">${esc(theme.label)}</span>
      <span class="theme-choice-meta">${esc(theme.family)}</span>
    </button>`).join('');
  root.querySelectorAll('[data-theme-choice]').forEach((choice) => {
    choice.onclick = (event) => {
      event.preventDefault();
      event.stopPropagation();
      applyTheme(choice.dataset.themeChoice);
      toggleThemePop(false);
      toast(`已切换到${themeLabel(state.theme)}`);
    };
  });
  syncThemeChrome();
}
function toggleThemePop(force) {
  if (state.mode === 'capsule') return;
  const pop = $('themePop');
  if (!pop) return;
  const target = force !== undefined ? force : !state.themeOpen;
  if (target) {
    if (state.menuOpen) {
      state.menuOpen = false;
      syncMenuChrome();
    }
    state.themeOpen = true;
    focusReturnTo = $('btnTheme');
    syncThemeChrome();
    setTimeout(() => moveFocusInto(pop), 50);
  } else {
    state.themeOpen = false;
    syncThemeChrome();
    restoreFocus();
  }
}
function syncMenuChrome() {
  const pop = $('menuPop');
  const scrim = $('menuScrim');
  const btn = $('btnMenu');
  if (pop) pop.hidden = !state.menuOpen;
  if (scrim) scrim.hidden = !state.menuOpen;
  if (btn) {
    btn.classList.toggle('on', state.menuOpen);
    btn.setAttribute('aria-expanded', String(state.menuOpen));
  }
  /* 开关行状态（与胶囊文案对齐：自动跟随 / 窗口置顶） */
  const auto = $('btnAutoFollow');
  if (auto) {
    auto.classList.toggle('on', !!state.autoFollowImportant);
    auto.setAttribute('aria-checked', String(!!state.autoFollowImportant));
    const mark = auto.querySelector('i');
    if (mark) mark.textContent = state.autoFollowImportant ? '开' : '关';
  }
  const top = $('btnTop');
  if (top) {
    top.classList.toggle('on', !!state.alwaysOnTop);
    top.setAttribute('aria-checked', String(!!state.alwaysOnTop));
    const mark = top.querySelector('i');
    if (mark) mark.textContent = state.alwaysOnTop ? '开' : '关';
  }
  const adm = $('btnAdmin');
  if (adm) adm.setAttribute('aria-expanded', String(!!state.adminOpen));
  const treeBtn = $('btnTree');
  if (treeBtn) {
    treeBtn.classList.toggle('on', !!state.treeOpen);
    treeBtn.setAttribute('aria-expanded', String(!!state.treeOpen));
  }
  syncThemeChrome();
}
function toggleMenu(force) {
  if (state.mode === 'capsule') return; /* 胶囊模式只允许胶囊菜单 */
  const target = force !== undefined ? force : !state.menuOpen;
  if (target) {
    if (state.themeOpen) toggleThemePop(false);
    closeProjectPop();
    /* 开菜单时不叠 sheet：先收起 */
    if (state.treeOpen || state.adminOpen) {
      state.treeOpen = false;
      state.adminOpen = false;
    }
    state.menuOpen = true;
    syncMenuChrome();
    focusReturnTo = $('btnMenu');
    setTimeout(() => moveFocusInto($('menuPop')), 50);
    render(); /* 刷新 sheet 类名 */
  } else {
    state.menuOpen = false;
    syncMenuChrome();
    restoreFocus();
    scheduleWindowFit();
  }
}

/* ---------- 事件绑定 ---------- */
function trapTabIn(panel, e) {
  if (!panel || e.key !== 'Tab') return false;
  /* [href] 会误匹配 <use href="icons.svg#..."> 图标引用；use 不是可聚焦控件，排除（SVG tagName 小写） */
  const items = [...panel.querySelectorAll('button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])')]
    .filter((el) => el.tagName.toLowerCase() !== 'use' && !el.disabled && el.offsetParent !== null);
  if (!items.length) return false;
  const first = items[0];
  const last = items[items.length - 1];
  const active = document.activeElement;
  if (e.shiftKey && (active === first || !panel.contains(active))) {
    e.preventDefault();
    last.focus();
    return true;
  }
  if (!e.shiftKey && (active === last || !panel.contains(active))) {
    e.preventDefault();
    first.focus();
    return true;
  }
  return false;
}
function bindUI() {
  renderThemePicker();
  $('btnScanExisting').onclick = () => addRootFlow(true);
  $('btnStartEmpty').onclick = () => startWithoutProject();
  $('btnRefresh').onclick = () => { toggleMenu(false); manualRefresh(); };
  $('btnTree').onclick = () => { if (state.menuOpen) toggleMenu(false); toggleTree(); };
  $('btnCapsule').onclick = () => { toggleMenu(false); setMode('capsule'); };
  $('btnAutoFollow').onclick = () => { toggleAutoFollow(); syncMenuChrome(); };
  $('btnTop').onclick = () => { toggleTop().then(() => syncMenuChrome()); };
  $('btnAdmin').onclick = () => { toggleMenu(false); toggleAdmin(); };
  if ($('btnAdminClose')) $('btnAdminClose').onclick = () => closeAdmin();
  if ($('btnTreeClose')) $('btnTreeClose').onclick = () => closeTree();
  /* 「显示已归档」勾选框：切换后立即重渲染任务树 */
  const chkArchived = $('chkShowArchived');
  if (chkArchived) {
    chkArchived.checked = !!state.showArchived;
    chkArchived.onchange = () => {
      state.showArchived = chkArchived.checked;
      savePrefs();
      if (state.view === 'main') render();
    };
  }
  $('btnHide').onclick = () => { toggleMenu(false); hideWindow(); };
  $('btnFlip').onclick = () => { toggleMenu(false); closeProjectPop(); toggleFlip(); };
  $('btnMenu').onclick = (e) => { e.stopPropagation(); toggleMenu(); };
  $('btnTheme').onclick = (e) => {
    e.preventDefault();
    e.stopPropagation();
    toggleMenu(false);
    toggleThemePop();
  };
  if ($('btnProjectChip')) {
    $('btnProjectChip').onclick = (e) => { e.stopPropagation(); toggleProjectPop(); };
  }
  if ($('menuScrim')) {
    $('menuScrim').onclick = () => { if (state.menuOpen) toggleMenu(false); };
  }

  /* 胶囊分区交互：内容回卡片 / 菜单 / 未读 badge，拖动区无 click */
  const capBody = $('capBody');
  if (capBody) {
    capBody.onclick = (e) => {
      if (e.target.closest('#capBadge, .cap-badge')) return;
      toggleCapMenu(false);
      setMode('card');
    };
  }
  const capBadge = $('capBadge');
  if (capBadge) {
    capBadge.onclick = (e) => {
      e.preventDefault();
      e.stopPropagation();
      toggleCapMenu(false);
      openUnreadFromCapsule().catch((err) => console.error('[cap-badge]', err));
    };
  }
  const btnCapMenu = $('btnCapMenu');
  if (btnCapMenu) {
    btnCapMenu.onclick = (e) => {
      e.preventDefault();
      e.stopPropagation();
      toggleCapMenu();
    };
  }
  const btnCapToCard = $('btnCapToCard');
  if (btnCapToCard) btnCapToCard.onclick = (e) => { e.stopPropagation(); toggleCapMenu(false); setMode('card'); };
  const btnCapLock = $('btnCapLock');
  if (btnCapLock) {
    btnCapLock.onclick = (e) => {
      e.stopPropagation();
      if (state.focusMode === 'manual') unlockFocus();
      else {
        const t = currentFocus();
        if (t) focusTask(t);
        else { state.focusMode = 'manual'; state.focusLockUntil = 0; render(); }
      }
      toggleCapMenu(false);
      syncCapMenu();
    };
  }
  const btnCapAuto = $('btnCapAutoFollow');
  if (btnCapAuto) {
    btnCapAuto.onclick = (e) => {
      e.stopPropagation();
      toggleAutoFollow();
      toggleCapMenu(false);
      syncCapMenu();
    };
  }
  const btnCapTop = $('btnCapTop');
  if (btnCapTop) {
    btnCapTop.onclick = (e) => {
      e.stopPropagation();
      toggleTop().then(() => {
        toggleCapMenu(false);
        syncCapMenu();
      });
    };
  }
  const btnCapHide = $('btnCapHide');
  if (btnCapHide) btnCapHide.onclick = (e) => { e.stopPropagation(); toggleCapMenu(false); hideWindow(); };

  document.addEventListener('click', (e) => {
    if (state.menuOpen && !e.target.closest('#menuPop, #btnMenu, #menuScrim')) {
      /* scrim 自己处理；这里兜底 */
      if (!e.target.closest('#menuScrim')) toggleMenu(false);
    }
    if (isProjectPopOpen() && !e.target.closest('#projectPop, #btnProjectChip')) {
      closeProjectPop();
      restoreFocus();
    }
    if (state.themeOpen && !e.target.closest('#themePop, #btnTheme')) toggleThemePop(false);
    const capPop = $('capMenuPop');
    if (capPop && !capPop.hidden && !e.target.closest('#capMenuPop, #btnCapMenu')) toggleCapMenu(false);
  });
  /* 点击卡片空白区翻面；避让顶栏/菜单/sheet */
  $('flip').addEventListener('click', (e) => {
    if (state.mode !== 'card' || state.treeOpen || state.adminOpen || state.menuOpen) return;
    if (e.target.closest('.subs, button, a, details, .doc, .runtime-evidence, .arts-mini, .dtabs, .sheet, .dragbar, input, .card-topbar, .menu-pop, .project-pop, .excerpt')) return;
    toggleFlip();
  });

  document.addEventListener('keydown', (e) => {
    const capPop = $('capMenuPop');
    const capMenuOpen = !!(capPop && !capPop.hidden);
    const menuPop = $('menuPop');
    const projectPop = $('projectPop');
    const themePop = $('themePop');

    if (state.themeOpen && (e.key === 'Tab' || e.key === 'Escape')) {
      if (e.key === 'Escape') { e.preventDefault(); toggleThemePop(false); return; }
      if (trapTabIn(themePop, e)) return;
      return;
    }

    if (capMenuOpen && (e.key === 'Tab' || e.key === 'Escape')) {
      if (e.key === 'Escape') { e.preventDefault(); toggleCapMenu(false); return; }
      if (trapTabIn(capPop, e)) return;
      return;
    }
    if (state.menuOpen && (e.key === 'Tab' || e.key === 'Escape')) {
      if (e.key === 'Escape') { e.preventDefault(); toggleMenu(false); return; }
      if (trapTabIn(menuPop, e)) return;
      return;
    }
    if (isProjectPopOpen() && (e.key === 'Tab' || e.key === 'Escape')) {
      if (e.key === 'Escape') { e.preventDefault(); closeProjectPop(); restoreFocus(); return; }
      if (trapTabIn(projectPop, e)) return;
      return;
    }
    if (state.adminOpen && e.key === 'Tab') {
      if (trapTabIn($('admin'), e)) return;
    }
    if (state.treeOpen && e.key === 'Tab') {
      if (trapTabIn($('list'), e)) return;
    }

    if (e.target && ['INPUT', 'TEXTAREA'].includes(e.target.tagName)) return;
    const k = e.key;
    if (state.view !== 'main') return;
    if (k === 'Escape') {
      if (state.adminOpen) { closeAdmin(); }
      else if (state.treeOpen) { closeTree(); }
      else if (state.flipped) { toggleFlip(false); }
      return;
    }
    if (k === 'l' || k === 'L') toggleTree();
    if (k === 'c' || k === 'C') setMode(state.mode === 'capsule' ? 'card' : 'capsule');
    if (k === 'r' || k === 'R') manualRefresh();
    if (state.treeOpen && k >= '1' && k <= '9') {
      const t = indexedTasks[Number(k) - 1];
      if (t) focusTask(t);
    }
  });

  document.addEventListener('visibilitychange', () => {
    if (!document.hidden && state.view === 'main') refresh(true);
  });
}

/* ---------- 摘要点击展开/收起（不再 hover 触发，避免窗口被拉伸） ---------- */
function bindExcerptFit() {
  document.addEventListener('click', (e) => {
    const excerpt = e.target.closest && e.target.closest('.excerpt');
    if (!excerpt) return;
    excerpt.classList.toggle('expanded');
    /* 展开后窗口高度跟随（防抖一次性 resize）；收起恢复 */
    scheduleWindowFit();
  });
  /* 原生窗口 resize 完成后，WebView 的 innerHeight 才会更新；重新测量
     可避免卡片仍按 resize 前的视口高度布局，导致底部出现透明空白。 */
  window.addEventListener('resize', () => {
    if (state.view === 'main' && state.mode === 'card' && !state.treeOpen) {
      scheduleWindowFit();
    }
  });
}

/* ---------- 启动 ---------- */
async function boot() {
  loadPrefs();
  bindUI();
  bindExcerptFit();
  let cfg = null;
  try {
    cfg = await call('get_config');
  } catch (e) {
    console.error('[invoke:get_config]', e);
    toast(hasTauri ? '读取配置失败' : '未在 Tauri 环境中运行');
  }
  if (cfg) {
    state.roots = cfg.roots || [];
    state.alwaysOnTop = !!cfg.alwaysOnTop;   /* 后端为权威来源 */
    state.configured = !!cfg.configured;
  }
  if (state.configured) {
    state.view = 'main';
    $('setup').hidden = true;
    $('mainView').hidden = false;
    /* 启动后强制对齐一次窗口模式，避免状态与窗口尺寸不一致 */
    if (hasTauri) call('set_window_mode', { mode: state.mode }).catch(() => {});
    await refresh(true);
    /* 先以 card 完成首屏 render（含 GSAP），再切 capsule，避免 .pane 不存在的警告 */
    render();
  } else {
    state.view = 'setup';
    renderSetup();
  }
  setInterval(pollTasks, POLL_MS);   /* 轮询兜底：文件监听之外的保险 */
  bindWatch();
}

boot();
