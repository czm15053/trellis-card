(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.TrellisFocusPolicy = api;
}(typeof globalThis === 'object' ? globalThis : this, function () {
  const IMPORTANT_STATES = new Set([
    'waiting_permission',
    'waiting_question',
    'blocked',
    'failed',
    'completed',
    'turn_done',
  ]);
  const LIVE_ACTIVITY_STATES = new Set([
    'working',
    'waiting_permission',
    'waiting_question',
    'blocked',
    'failed',
  ]);

  function isImportantRuntimeState(displayState) {
    return IMPORTANT_STATES.has(displayState);
  }

  function hasLiveRuntimeActivity(displayState) {
    return LIVE_ACTIVITY_STATES.has(displayState);
  }

  function shouldAutoFocus({
    enabled,
    locked,
    hasCurrentFocus,
    nextState,
    filter,
    nextProject,
  }) {
    if (!nextState) return false;
    if (filter && filter !== nextProject) return false;
    if (!hasCurrentFocus) return true;
    if (!enabled || locked) return false;
    return hasLiveRuntimeActivity(nextState);
  }

  // 未读去重：同一候选 key 且 lastChangedAt 未变时，轮询不应重复记未读。
  // 返回值 true 表示这是一个「新」的被拒候选，应记一次未读。
  function shouldRecordUnread({ candidateKey, lastChangedAt, prevKey, prevStamp }) {
    if (!candidateKey) return false;
    if (candidateKey !== prevKey) return true;
    return (lastChangedAt || 0) !== (prevStamp || 0);
  }

  // 未读去重状态机：维护最近一次已记未读的 (key, stamp)。
  // 返回 { record, nextKey, nextStamp }。同一 key + 同 stamp 连续 apply 只记一次，
  // 这样 focus-task-changed 与 agent-state-changed/轮询即使带同一快照也不会重复计数。
  function unreadDedup(prevKey, prevStamp, candidateKey, lastChangedAt) {
    const nextStamp = lastChangedAt || 0;
    if (!candidateKey) return { record: false, nextKey: prevKey, nextStamp: prevStamp };
    if (candidateKey !== prevKey || nextStamp !== (prevStamp || 0)) {
      return { record: true, nextKey: candidateKey, nextStamp };
    }
    return { record: false, nextKey: prevKey, nextStamp: prevStamp };
  }

  // 查看活动：返回当前点击是否可以回到对应任务的正面活动卡。
  function resolveUnreadAction({ mode, treeOpen, adminOpen, hasCurrentFocus }) {
    const canOpen = mode === 'card' && !treeOpen && !adminOpen && Boolean(hasCurrentFocus);
    return { shouldOpenActivityCard: canOpen };
  }

  // 完成态候选：completed/turn_done。
  function isDoneState(displayState) {
    return displayState === 'completed' || displayState === 'turn_done';
  }

  // 分类 runtime 候选：'adopt'（应切换焦点）/ 'refresh-same'（已是当前任务，原地刷新，不记未读）/ 'reject'（被拒，记未读）。
  function classifyRuntimeCandidate({
    enabled,
    locked,
    hasCurrentFocus,
    currentKey,
    candidateKey,
    nextState,
    filter,
    nextProject,
    isNewSinceEnabled,
  }) {
    if (!candidateKey) return 'reject';
    // 重要更新发生在本任务上：原地刷新状态/活动，不记为未读，也不切换（本来就是当前任务）。
    if (hasCurrentFocus && currentKey === candidateKey) return 'refresh-same';
    // 完成态候选且非当前焦点：不得 adopt（完成态自动推进已把焦点切走，
    // 后续 snapshot 的 focusKey 若仍指向已完成任务，不得把用户拉回）。
    if (hasCurrentFocus && currentKey !== candidateKey && isDoneState(nextState)) return 'reject';
    if (filter && filter !== nextProject) return 'reject';
    if (!hasCurrentFocus) return 'adopt';
    if (!enabled || locked) return 'reject';
    // Show live agent activity immediately. Completion states stay on the
    // existing completion/advance path instead of pulling focus back.
    if (!hasLiveRuntimeActivity(nextState)) return 'reject';
    if (!isNewSinceEnabled) return 'reject';
    return 'adopt';
  }

  // loadBack 异步回填判断：翻转状态 + 本次目标 key 匹配 + (evidence 或 focus) 任务 key 匹配。
  // evidence 候选非 currentFocus 时必须能回填，避免停在「加载中」。
  function shouldBackfillBack({ flipped, activeKey, targetKey, curKey }) {
    if (!flipped) return false;
    if (activeKey !== targetKey) return false;
    return curKey === targetKey;
  }

  // 项目级 activity 是否允许覆盖主卡：
  // 只有当没有可关联的 task runtime view/当前任务候选时，才允许显示项目级卡片。
  // blocked/failed/waiting 等重要任务态即使没有 agent session，也优先于项目级会话。
  function shouldShowProjectActivity({
    hasTaskCandidate,
    hasImportantTaskView,
    hasTaskSessionOnProject,
  }) {
    if (hasTaskCandidate) return false;
    if (hasImportantTaskView) return false;
    if (hasTaskSessionOnProject) return false;
    return true;
  }

  // 新任务创建后是否应自动聚焦。
  // runtimeFocusKey 为空：普通新任务创建，允许自动切换；
  // runtimeFocusKey 等于新任务：Hook 已先把运行时焦点指到它，仍应切换主卡；
  // runtimeFocusKey 指向别的任务：避免新任务覆盖正在处理的重要 runtime 候选。
  function shouldFocusNewTaskOnCreate({
    enabled,
    locked,
    runtimeFocusKey,
    newTaskKey,
  }) {
    if (!enabled || locked || !newTaskKey) return false;
    return !runtimeFocusKey || runtimeFocusKey === newTaskKey;
  }

  // 完成态自动推进：当前 focus 任务是否「刚完成且应自动切换」。
  // 归档是用户明确结束当前任务的动作，交给前端保留归档回执，不自动抢焦点。
  // 其他情况仅当 自动跟随开启 + 非手动锁定 + 当前有 focus 任务 + prevState 非完成态
  // + nextState 为 completed/turn_done 时才应推进。
  function shouldAutoAdvanceOnCompletion({
    enabled,
    locked,
    hasCurrentFocus,
    prevState,
    nextState,
    action,
  }) {
    if (!hasCurrentFocus) return false;
    if (!enabled || locked) return false;
    if (action === 'archive') return false;
    if (prevState === 'completed' || prevState === 'turn_done') return false;
    return nextState === 'completed' || nextState === 'turn_done';
  }

  // 完成态自动推进：选下一个进行中任务。
  // 候选须 unfinished（由调用方过滤传入），排除 currentKey（刚完成的任务）。
  // 候选跨项目：完成/归档后焦点跳到「最近有变化」的任务，不受项目边界限制。
  // 排序：lastChangedAt（runtime 活动变化时间，含 AI 会话与项目活动）降序优先，
  // 无 lastChangedAt 时按 mtime 降序兜底。时间值由调用方预计算注入，保持本函数纯。
  // 返回选中候选的 key，无候选返回 null。
  function nextFocusAfterCompletion({ candidates, currentKey }) {
    if (!Array.isArray(candidates) || !candidates.length) return null;
    const withoutCurrent = candidates.filter(
      (c) => c.key !== currentKey && !c.completed,
    );
    if (!withoutCurrent.length) return null;
    return withoutCurrent.sort(
      (a, b) =>
        (b.lastChangedAt || 0) - (a.lastChangedAt || 0) ||
        (b.mtime || 0) - (a.mtime || 0),
    )[0].key;
  }

  return {
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
  };
}));
