const test = require('node:test');
const assert = require('node:assert/strict');
const { semanticizeActivity } = require('../src/activity-display.js');

const phraseGroups = {
  reading: /检查中…|审阅中…|解析中…|审计中…|加载中…/,
  writing: /重构中…|实现中…|修补中…|搭建中…|提交中…/,
  running: /编译中…|构建中…|执行中…|部署中…|运行流水线中…/,
  searching: /扫描中…|搜索中…|建立索引中…|追踪中…|分析性能中…/,
  delegating: /启动进程中…|分叉中…|派发中…|排队任务中…/,
  skill: /阅读手册中…|查看文档中…|加载模块中…/,
};

test('按 AgentPet 的工具关键词顺序分类', () => {
  const cases = [
    ['Read', 'reading'], ['Edit', 'writing'], ['Bash', 'running'],
    ['Glob', 'searching'], ['Grep', 'searching'], ['WebSearch', 'searching'],
    ['Agent', 'delegating'], ['Task', 'delegating'], ['Skill', 'skill'],
  ];
  for (const [tool, group] of cases) {
    assert.match(
      semanticizeActivity('', tool, null, { eventName: 'PreToolUse' }),
      phraseGroups[group],
      `${tool} should use the ${group} phrase pool`,
    );
  }
});

test('使用结构化 filePath 生成 AgentPet 风格的文件提示', () => {
  assert.equal(
    semanticizeActivity('', 'Read', null, {
      eventName: 'PreToolUse',
      toolInput: { filePath: 'Tests/FooTests/BarTests.swift' },
    }),
    '检查测试中…',
  );
  assert.equal(
    semanticizeActivity('', 'Edit', null, {
      eventName: 'PreToolUse',
      toolInput: { filePath: 'docs/guide.md' },
    }),
    '更新文档中…',
  );
  assert.equal(
    semanticizeActivity('', 'Read', null, {
      eventName: 'PreToolUse',
      toolInput: { filePath: 'config/settings.json' },
    }),
    '解析配置中…',
  );
});

test('不再从命令内容猜测测试、提交或部署', () => {
  const text = semanticizeActivity('cargo test', 'Bash', 'implement', {
    eventName: 'PreToolUse',
    toolInput: { command: 'cargo test' },
  });
  assert.match(text, phraseGroups.running);
  assert.notEqual(text, '运行测试');
  assert.notEqual(text, '实现任务');
});

test('事件文案遵循 AgentPet 的生命周期规则', () => {
  assert.match(
    semanticizeActivity('用户要求', null, null, { eventName: 'UserPromptSubmit' }),
    /设计架构中…|设计中…|计算中…|调试中…|优化中…/,
  );
  assert.match(
    semanticizeActivity('', null, null, { eventName: 'PermissionRequest' }),
    /等待输入…|依赖未就绪…|轮询中…/,
  );
  assert.match(
    semanticizeActivity('', null, null, { eventName: 'Stop' }),
    /构建完成！|已发布！|已合并！|全部通过！|测试通过！/,
  );
  assert.equal(
    semanticizeActivity('需要用户确认', null, null, { eventName: 'Notification' }),
    '需要用户确认',
  );
});

test('Trellis 原生工具走精确语义，不落入通用短语池', () => {
  const cases = [
    ['trellis-implement', '编码实现中…'],
    ['trellis-check', '验证与自修复中…'],
    ['trellis-brainstorm', '头脑风暴 · 澄清需求'],
    ['trellis-research', '调研 · 收集证据'],
    ['trellis-archive', '归档任务'],
    ['trellis:start', '开启 Trellis 会话'],
    ['trellis:finish-work', '收尾并归档任务'],
    ['trellis:continue', '推进任务下一步'],
  ];
  for (const [tool, expected] of cases) {
    assert.equal(
      semanticizeActivity('', tool, null, { eventName: 'PreToolUse' }),
      expected,
      `${tool} 应精确翻译`,
    );
  }
});

test('task.py 子命令从 command 提取精确语义', () => {
  const cases = [
    ['trellis-implement', 'python3 .trellis/scripts/task.py create "新增登录"', '创建任务'],
    ['Bash', 'task.py start 07-demo', '激活任务'],
    ['Bash', './.trellis/scripts/task.py archive user-login', '归档任务'],
    ['trellis-check', 'task.py add-context /repo spec index.md', '注入任务上下文'],
  ];
  for (const [tool, command, expected] of cases) {
    assert.equal(
      semanticizeActivity('', tool, null, { eventName: 'PreToolUse', toolInput: { command } }),
      expected,
    );
  }
});

test('trellis CLI 命令从 command 提取精确语义', () => {
  const cases = [
    ['Bash', 'trellis upgrade', '升级 Trellis CLI'],
    ['Bash', 'trellis update', '同步项目到 CLI 版本'],
    ['Bash', 'trellis init', '初始化 Trellis 项目'],
  ];
  for (const [tool, command, expected] of cases) {
    assert.equal(
      semanticizeActivity('', tool, null, { eventName: 'PreToolUse', toolInput: { command } }),
      expected,
    );
  }
});

test('没有工具或事件时保留原始活动', () => {
  assert.equal(semanticizeActivity('请检查这个任务的实现', null), '请检查这个任务的实现');
});
