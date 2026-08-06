const test = require('node:test');
const assert = require('node:assert/strict');
const {
  isImportantRuntimeState,
  shouldAutoFocus,
  shouldRecordUnread,
  unreadDedup,
  resolveUnreadAction,
  classifyRuntimeCandidate,
  shouldBackfillBack,
  shouldShowProjectActivity,
  shouldFocusNewTaskOnCreate,
  shouldAutoAdvanceOnCompletion,
  nextFocusAfterCompletion,
} = require('../src/focus-policy.js');

test('只有需要处理或结果状态才是重要状态', () => {
  assert.equal(isImportantRuntimeState('waiting_permission'), true);
  assert.equal(isImportantRuntimeState('blocked'), true);
  assert.equal(isImportantRuntimeState('turn_done'), true);
  assert.equal(isImportantRuntimeState('working'), false);
  assert.equal(isImportantRuntimeState('reviewing'), false);
});

test('没有当前任务时允许首次聚焦', () => {
  assert.equal(shouldAutoFocus({
    enabled: true,
    locked: false,
    hasCurrentFocus: false,
    nextState: 'working',
    filter: null,
    nextProject: 'alpha',
  }), true);
});

test('开启自动跟随时重要状态可以切换焦点', () => {
  assert.equal(shouldAutoFocus({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    nextState: 'waiting_permission',
    filter: null,
    nextProject: 'alpha',
  }), true);
});

test('普通 working 实时活动可以抢焦点', () => {
  assert.equal(shouldAutoFocus({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    nextState: 'working',
    filter: null,
    nextProject: 'alpha',
  }), true);
});

test('有实时 working 活动时直接切换显示，不降级为未读', () => {
  assert.equal(classifyRuntimeCandidate({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    currentKey: 'alpha::current',
    candidateKey: 'alpha::active',
    nextState: 'working',
    filter: null,
    nextProject: 'alpha',
    isNewSinceEnabled: true,
  }), 'adopt');
});

test('关闭自动跟随或手动锁定后都不切换', () => {
  for (const extra of [{ enabled: false }, { locked: true }]) {
    assert.equal(shouldAutoFocus({
      enabled: true,
      locked: false,
      hasCurrentFocus: true,
      nextState: 'blocked',
      filter: null,
      nextProject: 'alpha',
      ...extra,
    }), false);
  }
});

test('其他项目不能越过当前筛选抢焦点', () => {
  assert.equal(shouldAutoFocus({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    nextState: 'blocked',
    filter: 'gamma',
    nextProject: 'alpha',
  }), false);
});

test('手动锁定优先于自动跟随：locked 时重要状态也不切换', () => {
  assert.equal(shouldAutoFocus({
    enabled: true,
    locked: true,
    hasCurrentFocus: true,
    nextState: 'waiting_permission',
    filter: null,
    nextProject: 'alpha',
  }), false);
});

test('筛选下同项目重要状态仍可切换', () => {
  assert.equal(shouldAutoFocus({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    nextState: 'blocked',
    filter: 'alpha',
    nextProject: 'alpha',
  }), true);
});

test('manual 锁定 + 自动跟随关闭：任何候选都不采纳（必走未读分支）', () => {
  for (const nextState of ['blocked', 'waiting_permission', 'completed']) {
    assert.equal(shouldAutoFocus({
      enabled: false,
      locked: true,
      hasCurrentFocus: true,
      nextState,
      filter: null,
      nextProject: 'alpha',
    }), false);
  }
});

/* ---- shouldRecordUnread 去重回归：轮询同候选不应重复记未读 ---- */

test('新候选 key 应记未读', () => {
  assert.equal(shouldRecordUnread({
    candidateKey: 'alpha::task-a',
    lastChangedAt: 100,
    prevKey: null,
    prevStamp: 0,
  }), true);
});

test('同一候选 key 且 lastChangedAt 未变：轮询不重复记未读', () => {
  assert.equal(shouldRecordUnread({
    candidateKey: 'alpha::task-a',
    lastChangedAt: 100,
    prevKey: 'alpha::task-a',
    prevStamp: 100,
  }), false);
});

test('同一候选 key 但 lastChangedAt 变化（新 activity）：再记一次未读', () => {
  assert.equal(shouldRecordUnread({
    candidateKey: 'alpha::task-a',
    lastChangedAt: 200,
    prevKey: 'alpha::task-a',
    prevStamp: 100,
  }), true);
});

test('候选 key 变化：即使时间戳相同也记未读', () => {
  assert.equal(shouldRecordUnread({
    candidateKey: 'alpha::task-b',
    lastChangedAt: 100,
    prevKey: 'alpha::task-a',
    prevStamp: 100,
  }), true);
});

test('无候选 key：不记未读', () => {
  assert.equal(shouldRecordUnread({
    candidateKey: null,
    lastChangedAt: 0,
    prevKey: null,
    prevStamp: 0,
  }), false);
});

/* ---- unreadDedup 状态机回归：双事件 / 轮询重复快照只记一次 ---- */

test('同一快照连续两次 apply（focus-task-changed 旧值 + agent-state-changed 新值）只记一次未读', () => {
  /* 第一次：focus-task-changed 以旧 runtimeByTask（stamp=100）进入，先记一次 */
  let s1 = unreadDedup(null, 0, 'alpha::task-a', 100);
  assert.equal(s1.record, true);
  /* 第二次：agent-state-changed 拿到新 lastChangedAt=100（同快照），不重复记 */
  const s2 = unreadDedup(s1.nextKey, s1.nextStamp, 'alpha::task-a', 100);
  assert.equal(s2.record, false);
});

test('新 activity（lastChangedAt 变化）即使同一 key 也再记未读', () => {
  let s1 = unreadDedup(null, 0, 'alpha::task-a', 100);
  assert.equal(s1.record, true);
  const s2 = unreadDedup(s1.nextKey, s1.nextStamp, 'alpha::task-a', 200);
  assert.equal(s2.record, true);
});

test('焦点切换采纳后重置标记：同 key 后续被拒仍能记未读', () => {
  /* 采纳后 reset 到 (null, 0)，随后同一 key 被拒应再记一次 */
  let s1 = unreadDedup(null, 0, 'alpha::task-a', 100);
  assert.equal(s1.record, true);
  const s2 = unreadDedup(null, 0, 'alpha::task-a', 100);
  assert.equal(s2.record, true);
});

test('跨 key 轮询：key 变化即记未读，随后同 key 同 stamp 不重复', () => {
  let s1 = unreadDedup(null, 0, 'alpha::task-a', 100);
  assert.equal(s1.record, true);
  let s2 = unreadDedup(s1.nextKey, s1.nextStamp, 'alpha::task-b', 100);
  assert.equal(s2.record, true);
  const s3 = unreadDedup(s2.nextKey, s2.nextStamp, 'alpha::task-b', 100);
  assert.equal(s3.record, false);
});

test('无候选 key：状态机保持原标记且不记未读', () => {
  const s = unreadDedup('alpha::task-a', 100, null, 0);
  assert.equal(s.record, false);
  assert.equal(s.nextKey, 'alpha::task-a');
  assert.equal(s.nextStamp, 100);
});

/* ---- resolveUnreadAction 回归：查看活动按钮行为 ---- */

test('查看活动：卡片模式 + 无 sheet + 有焦点 → 打开未读活动卡', () => {
  const a = resolveUnreadAction({ mode: 'card', treeOpen: false, adminOpen: false, hasCurrentFocus: true });
  assert.equal(a.shouldOpenActivityCard, true);
});

test('查看活动：胶囊模式 → 不打开未读活动卡', () => {
  const a = resolveUnreadAction({ mode: 'capsule', treeOpen: false, adminOpen: false, hasCurrentFocus: true });
  assert.equal(a.shouldOpenActivityCard, false);
});

test('查看活动：sheet 打开 → 不打开未读活动卡', () => {
  const a = resolveUnreadAction({ mode: 'card', treeOpen: true, adminOpen: false, hasCurrentFocus: true });
  assert.equal(a.shouldOpenActivityCard, false);
});

test('查看活动：无当前焦点 → 不打开未读活动卡', () => {
  const a = resolveUnreadAction({ mode: 'card', treeOpen: false, adminOpen: false, hasCurrentFocus: false });
  assert.equal(a.shouldOpenActivityCard, false);
});

/* ---- classifyRuntimeCandidate 回归：same-task / adopt / reject ---- */

const baseCandidate = (extra = {}) => ({
  enabled: true,
  locked: false,
  hasCurrentFocus: true,
  currentKey: 'alpha::task-a',
  candidateKey: 'alpha::task-b',
  nextState: 'waiting_permission',
  filter: null,
  nextProject: 'alpha',
  isNewSinceEnabled: true,
  ...extra,
});

test('分类：重要更新发生在当前已聚焦任务 → refresh-same（原地刷新，不记未读）', () => {
  assert.equal(classifyRuntimeCandidate(baseCandidate({
    candidateKey: 'alpha::task-a', nextState: 'blocked',
  })), 'refresh-same');
});

test('分类：自动跟随开启 + 重要状态 + 非当前任务 → adopt', () => {
  assert.equal(classifyRuntimeCandidate(baseCandidate()), 'adopt');
});

test('分类：手动锁定 → reject（记未读，不切焦点）', () => {
  assert.equal(classifyRuntimeCandidate(baseCandidate({ locked: true })), 'reject');
});

test('分类：自动跟随关闭 → reject', () => {
  assert.equal(classifyRuntimeCandidate(baseCandidate({ enabled: false })), 'reject');
});

test('分类：普通 working 状态 → adopt（实时活动直接显示）', () => {
  assert.equal(classifyRuntimeCandidate(baseCandidate({ nextState: 'working' })), 'adopt');
});

test('分类：其他项目越过筛选 → reject', () => {
  assert.equal(classifyRuntimeCandidate(baseCandidate({ filter: 'gamma', nextProject: 'alpha' })), 'reject');
});

test('分类：非当前项目但重要且无筛选 → adopt', () => {
  assert.equal(classifyRuntimeCandidate(baseCandidate({ filter: null, nextProject: 'gamma' })), 'adopt');
});

test('分类：无候选 key → reject', () => {
  assert.equal(classifyRuntimeCandidate(baseCandidate({ candidateKey: null })), 'reject');
});

test('failed 属于重要状态', () => {
  assert.equal(isImportantRuntimeState('failed'), true);
});

/* ---- shouldBackfillBack 回归：查看活动候选详情不能停在加载中 ---- */

test('回填：evidence 候选（非 currentFocus）翻转后能回填详情', () => {
  /* evidence 候选 key = alpha::task-b，currentFocus = alpha::task-a，但展示目标就是候选 */
  assert.equal(shouldBackfillBack({
    flipped: true,
    activeKey: 'alpha::task-b',
    targetKey: 'alpha::task-b',
    curKey: 'alpha::task-b',
  }), true);
});

test('回填：未翻转（正面）不回填', () => {
  assert.equal(shouldBackfillBack({
    flipped: false,
    activeKey: 'alpha::task-b',
    targetKey: 'alpha::task-b',
    curKey: 'alpha::task-b',
  }), false);
});

test('回填：loadBack 本次目标 key 不匹配异步返回 key 时不回填', () => {
  assert.equal(shouldBackfillBack({
    flipped: true,
    activeKey: 'alpha::task-a',
    targetKey: 'alpha::task-a',
    curKey: 'alpha::task-b',
  }), false);
});

test('回填：焦点已切走（targetKey 不同）不回填', () => {
  assert.equal(shouldBackfillBack({
    flipped: true,
    activeKey: 'alpha::task-b',
    targetKey: 'alpha::task-c',
    curKey: 'alpha::task-b',
  }), false);
});

/* ---- shouldShowProjectActivity：任务候选优先于项目级会话 ---- */

test('有任务候选时不显示项目级 activity', () => {
  assert.equal(shouldShowProjectActivity({
    hasTaskCandidate: true,
    hasImportantTaskView: false,
    hasTaskSessionOnProject: false,
  }), false);
});

test('有重要任务态（blocked/failed/waiting）时不显示项目级 activity', () => {
  assert.equal(shouldShowProjectActivity({
    hasTaskCandidate: false,
    hasImportantTaskView: true,
    hasTaskSessionOnProject: false,
  }), false);
});

test('同项目已有 task session 时不显示项目级 activity', () => {
  assert.equal(shouldShowProjectActivity({
    hasTaskCandidate: false,
    hasImportantTaskView: false,
    hasTaskSessionOnProject: true,
  }), false);
});

test('无任务候选且无重要态且无 task session 时允许项目级 activity', () => {
  assert.equal(shouldShowProjectActivity({
    hasTaskCandidate: false,
    hasImportantTaskView: false,
    hasTaskSessionOnProject: false,
  }), true);
});

/* ---- shouldFocusNewTaskOnCreate：新任务创建后的主卡聚焦 ---- */

test('新任务创建：runtime 尚无焦点时自动聚焦新任务', () => {
  assert.equal(shouldFocusNewTaskOnCreate({
    enabled: true,
    locked: false,
    runtimeFocusKey: null,
    newTaskKey: 'testtest::08-02-mc-style-game',
  }), true);
});

test('新任务创建：runtime 已指向同一新任务时仍自动聚焦', () => {
  assert.equal(shouldFocusNewTaskOnCreate({
    enabled: true,
    locked: false,
    runtimeFocusKey: 'testtest::08-02-mc-style-game',
    newTaskKey: 'testtest::08-02-mc-style-game',
  }), true);
});

test('新任务创建：runtime 指向其他任务时不抢焦点', () => {
  assert.equal(shouldFocusNewTaskOnCreate({
    enabled: true,
    locked: false,
    runtimeFocusKey: 'testtest::00-bootstrap-guidelines',
    newTaskKey: 'testtest::08-02-mc-style-game',
  }), false);
});

test('新任务创建：手动锁定或关闭自动跟随时不自动聚焦', () => {
  assert.equal(shouldFocusNewTaskOnCreate({
    enabled: false,
    locked: false,
    runtimeFocusKey: null,
    newTaskKey: 'testtest::08-02-mc-style-game',
  }), false);
  assert.equal(shouldFocusNewTaskOnCreate({
    enabled: true,
    locked: true,
    runtimeFocusKey: null,
    newTaskKey: 'testtest::08-02-mc-style-game',
  }), false);
});

/* ---- shouldAutoAdvanceOnCompletion：完成态自动推进判定 ---- */

test('非完成态进入 completed 且自动跟随开+非锁定 应推进', () => {
  assert.equal(shouldAutoAdvanceOnCompletion({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    prevState: 'working',
    nextState: 'completed',
  }), true);
});

test('非完成态进入 turn_done 且自动跟随开+非锁定 应推进', () => {
  assert.equal(shouldAutoAdvanceOnCompletion({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    prevState: 'working',
    nextState: 'turn_done',
  }), true);
});

test('归档完成不自动推进，交给归档回执保留当前任务', () => {
  assert.equal(shouldAutoAdvanceOnCompletion({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    prevState: 'working',
    nextState: 'completed',
    action: 'archive',
  }), false);
});

test('手动锁定（manual）时不自动推进', () => {
  assert.equal(shouldAutoAdvanceOnCompletion({
    enabled: true,
    locked: true,
    hasCurrentFocus: true,
    prevState: 'working',
    nextState: 'completed',
  }), false);
});

test('自动跟随关闭时不自动推进', () => {
  assert.equal(shouldAutoAdvanceOnCompletion({
    enabled: false,
    locked: false,
    hasCurrentFocus: true,
    prevState: 'working',
    nextState: 'completed',
  }), false);
});

test('本来就在完成态（prev 已 completed）不重复推进', () => {
  assert.equal(shouldAutoAdvanceOnCompletion({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    prevState: 'completed',
    nextState: 'completed',
  }), false);
});

test('未进入完成态（仍 working）不推进', () => {
  assert.equal(shouldAutoAdvanceOnCompletion({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    prevState: 'working',
    nextState: 'working',
  }), false);
});

test('无当前 focus 任务时不推进', () => {
  assert.equal(shouldAutoAdvanceOnCompletion({
    enabled: true,
    locked: false,
    hasCurrentFocus: false,
    prevState: 'working',
    nextState: 'completed',
  }), false);
});

/* ---- nextFocusAfterCompletion：选择下一个进行中任务 ---- */

test('选择下一个：排除当前已完成任务，按 lastChangedAt 降序', () => {
  const next = nextFocusAfterCompletion({
    candidates: [
      { key: 'alpha::done', completed: true, lastChangedAt: 100, mtime: 100 },
      { key: 'alpha::a', completed: false, lastChangedAt: 50, mtime: 50 },
      { key: 'alpha::b', completed: false, lastChangedAt: 90, mtime: 30 },
    ],
    currentKey: 'alpha::done',
  });
  assert.equal(next, 'alpha::b');
});

test('选择下一个：无活跃会话时按 mtime 最大', () => {
  const next = nextFocusAfterCompletion({
    candidates: [
      { key: 'alpha::a', completed: false, focusScore: 0, lastSeen: 0, mtime: 50 },
      { key: 'alpha::b', completed: false, focusScore: 0, lastSeen: 0, mtime: 80 },
      { key: 'alpha::c', completed: false, focusScore: 0, lastSeen: 0, mtime: 20 },
    ],
    currentKey: 'alpha::x',
  });
  assert.equal(next, 'alpha::b');
});

test('选择下一个：排除刚完成的 currentKey', () => {
  const next = nextFocusAfterCompletion({
    candidates: [
      { key: 'alpha::just-done', completed: true, focusScore: 999, lastSeen: 999, mtime: 999 },
      { key: 'alpha::next', completed: false, focusScore: 100, lastSeen: 100, mtime: 100 },
    ],
    currentKey: 'alpha::just-done',
  });
  assert.equal(next, 'alpha::next');
});

test('选择下一个：无其他未完成任务时返回 null', () => {
  const next = nextFocusAfterCompletion({
    candidates: [
      { key: 'alpha::only-done', completed: true, focusScore: 0, lastSeen: 0, mtime: 1 },
    ],
    currentKey: 'alpha::only-done',
  });
  assert.equal(next, null);
});

test('选择下一个：空候选返回 null', () => {
  assert.equal(nextFocusAfterCompletion({ candidates: [], currentKey: 'x' }), null);
});

test('选择下一个：已完成的候选即使 lastSeen 高也被排除', () => {
  const next = nextFocusAfterCompletion({
    candidates: [
      { key: 'alpha::done-a', completed: true, focusScore: 500, lastSeen: 500, mtime: 500 },
      { key: 'alpha::done-b', completed: true, focusScore: 400, lastSeen: 400, mtime: 400 },
      { key: 'alpha::work-a', completed: false, focusScore: 10, lastSeen: 10, mtime: 10 },
    ],
    currentKey: 'alpha::done-a',
  });
  assert.equal(next, 'alpha::work-a');
});

test('选择下一个：跨项目候选中选 lastChangedAt 最大的', () => {
  /* beta 最近有变化，alpha 项目内候选变化较旧，应选 beta */
  const next = nextFocusAfterCompletion({
    candidates: [
      { key: 'alpha::done', completed: true, lastChangedAt: 500, mtime: 500 },
      { key: 'alpha::a', completed: false, lastChangedAt: 300, mtime: 300 },
      { key: 'beta::hot', completed: false, lastChangedAt: 999, mtime: 999 },
      { key: 'beta::cold', completed: false, lastChangedAt: 100, mtime: 100 },
    ],
    currentKey: 'alpha::done',
  });
  assert.equal(next, 'beta::hot');
});

test('选择下一个：跨项目选最近变化，不受项目边界限制', () => {
  /* alpha 项目内也有未完成任务，但 beta 变化更新，应跨项目选 beta */
  const next = nextFocusAfterCompletion({
    candidates: [
      { key: 'beta::fresh', completed: false, lastChangedAt: 800, mtime: 800 },
      { key: 'alpha::local', completed: false, lastChangedAt: 400, mtime: 400 },
    ],
    currentKey: 'alpha::done',
  });
  assert.equal(next, 'beta::fresh');
});

test('选择下一个：无 lastChangedAt 时按 mtime 降序兜底', () => {
  const next = nextFocusAfterCompletion({
    candidates: [
      { key: 'alpha::old', completed: false, lastChangedAt: 0, mtime: 30 },
      { key: 'beta::new', completed: false, lastChangedAt: 0, mtime: 80 },
      { key: 'alpha::mid', completed: false, lastChangedAt: 0, mtime: 50 },
    ],
    currentKey: 'alpha::just-done',
  });
  assert.equal(next, 'beta::new');
});

test('选择下一个：调用方已用 max(lastChangedAt, mtime) 兜底，纯函数按此排序', () => {
  /* app.js 侧已把 mtime 并入 lastChangedAt；候选 a 的 lastChangedAt 即 max(0, mtime) */
  const next = nextFocusAfterCompletion({
    candidates: [
      { key: 'beta::b', completed: false, lastChangedAt: 100, mtime: 100 },
      { key: 'alpha::a', completed: false, lastChangedAt: 200, mtime: 200 },
    ],
    currentKey: 'alpha::done',
  });
  assert.equal(next, 'alpha::a');
});

/* ---- 完成态候选不 adopt（焦点回跳防护） ---- */

test('完成态候选（非当前焦点）不被 adopt（后续 snapshot 不得拉回已完成任务）', () => {
  /* 完成推进已把焦点切到 work-a，snapshot focusKey 仍指向 done-a（completed） */
  assert.equal(classifyRuntimeCandidate({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    currentKey: 'alpha::work-a',
    candidateKey: 'alpha::done-a',
    nextState: 'completed',
    filter: null,
    nextProject: 'alpha',
    isNewSinceEnabled: true,
  }), 'reject');
});

test('完成态候选（非当前焦点，turn_done）不被 adopt', () => {
  assert.equal(classifyRuntimeCandidate({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    currentKey: 'alpha::work-a',
    candidateKey: 'alpha::done-b',
    nextState: 'turn_done',
    filter: null,
    nextProject: 'alpha',
    isNewSinceEnabled: true,
  }), 'reject');
});

test('完成态候选若是当前焦点则 refresh-same（原地刷新）', () => {
  assert.equal(classifyRuntimeCandidate({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    currentKey: 'alpha::done-a',
    candidateKey: 'alpha::done-a',
    nextState: 'completed',
    filter: null,
    nextProject: 'alpha',
    isNewSinceEnabled: true,
  }), 'refresh-same');
});

test('未完成重要态候选仍可 adopt（不破坏正常切换）', () => {
  assert.equal(classifyRuntimeCandidate({
    enabled: true,
    locked: false,
    hasCurrentFocus: true,
    currentKey: 'alpha::work-a',
    candidateKey: 'alpha::blocked-b',
    nextState: 'blocked',
    filter: null,
    nextProject: 'alpha',
    isNewSinceEnabled: true,
  }), 'adopt');
});

test('完成态候选在无当前焦点时仍可首次聚焦', () => {
  /* 无焦点（如首次扫描全是完成态）时允许 adopt，避免空态 */
  assert.equal(classifyRuntimeCandidate({
    enabled: true,
    locked: false,
    hasCurrentFocus: false,
    currentKey: null,
    candidateKey: 'alpha::done-a',
    nextState: 'completed',
    filter: null,
    nextProject: 'alpha',
    isNewSinceEnabled: true,
  }), 'adopt');
});
