(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.TrellisActivityDisplay = api;
}(typeof globalThis === 'object' ? globalThis : this, function () {
  /*
   * AgentPet 的 formatter 只做三件事：按工具类别选词、按结构化文件路径
   * 给出少量提示、按事件/状态选词。这里保留它的分类顺序和短语池，
   * 不再从命令内容猜测“正在测试/部署/提交”等具体语义。
   */
  const PHRASES = Object.freeze({
    reading: ['检查中…', '审阅中…', '解析中…', '审计中…', '加载中…'],
    writing: ['重构中…', '实现中…', '修补中…', '搭建中…', '提交中…'],
    running: ['编译中…', '构建中…', '执行中…', '部署中…', '运行流水线中…'],
    searching: ['扫描中…', '搜索中…', '建立索引中…', '追踪中…', '分析性能中…'],
    delegating: ['启动进程中…', '分叉中…', '派发中…', '排队任务中…'],
    thinking: ['设计架构中…', '设计中…', '计算中…', '调试中…', '优化中…'],
    done: ['构建完成！', '已发布！', '已合并！', '全部通过！', '测试通过！'],
    waiting: ['等待输入…', '依赖未就绪…', '轮询中…'],
    skill: ['阅读手册中…', '查看文档中…', '加载模块中…'],
    generic: ['处理中…', '运行中…', '执行中…'],
  });
  /* Trellis 命令体系（对照 docs.trytrellis.app/zh/start/everyday-use）：
     3 个 slash 命令 + native 工具（hook 注入的 tool_name / skill）。这些在文档中有
     明确语义，直接精确翻译，不走通用短语池。 */
  const TRELLIS_NATIVE = Object.freeze({
    'trellis:start': '开启 Trellis 会话',
    'trellis:continue': '推进任务下一步',
    'trellis:finish-work': '收尾并归档任务',
    'trellis-create': '创建 Trellis 任务',
    'trellis-brainstorm': '头脑风暴 · 澄清需求',
    'trellis-research': '调研 · 收集证据',
    'trellis-prd': '整理 PRD',
    'trellis-context': '补充任务上下文',
    'trellis-implement': '编码实现中…',
    'trellis-check': '验证与自修复中…',
    'trellis-rollback': '回滚改动',
    'trellis-break-loop': '根因分析与预防',
    'trellis-update-spec': '沉淀规范到 spec',
    'trellis-archive': '归档任务',
    'trellis-before-dev': '动手前读取规范',
  });
  /* task.py 子命令（文档 2.2 节）语义 */
  const TASK_SUBCOMMANDS = Object.freeze({
    create: '创建任务',
    'add-context': '注入任务上下文',
    start: '激活任务',
    finish: '结束任务会话',
    'set-branch': '设置分支',
    'set-base-branch': '设置基线分支',
    'set-scope': '设置 scope',
    list: '列出任务',
    'list-archive': '查看归档任务',
    archive: '归档任务',
    validate: '校验上下文引用',
    'add-subtask': '添加子任务',
    'remove-subtask': '解除子任务关联',
  });
  /* trellis CLI 命令（文档 1.2 节）语义 */
  const TRELLIS_CLI = Object.freeze({
    upgrade: '升级 Trellis CLI',
    update: '同步项目到 CLI 版本',
    init: '初始化 Trellis 项目',
  });
  const phraseCounters = new Map();

  function categoryForTool(toolName) {
    const name = String(toolName || '').trim().toLowerCase();
    if (!name) return 'generic';
    if (name.startsWith('trellis-') || name.startsWith('trellis:')) return 'trellis';
    if (name.includes('task') || name.includes('agent')) return 'delegating';
    if (name.includes('skill')) return 'skill';
    if (name.includes('search') || name.includes('grep') || name.includes('glob')
      || name.includes('find') || name.includes('list') || name.includes('fetch')) return 'searching';
    if (name.includes('run') || name.includes('shell') || name.includes('terminal')
      || name.includes('bash') || name.includes('exec') || name.includes('command')) return 'running';
    if (name.includes('edit') || name.includes('write') || name.includes('create')
      || name.includes('patch') || name.includes('delete')) return 'writing';
    if (name.includes('read') || name.includes('view')) return 'reading';
    return 'generic';
  }

  /* trellis native 工具 / skill：精确语义优先于通用短语池。 */
  function trellisNativeActivity(toolName) {
    const tool = String(toolName || '').trim().toLowerCase();
    if (TRELLIS_NATIVE[tool]) return TRELLIS_NATIVE[tool];
    return null;
  }

  /* task.py 子命令：从 command 提取（文档 2.2 节）。工具名可以是 trellis 原生工具或 Bash。 */
  function taskSubcommandActivity(toolInput) {
    const command = String((toolInput && toolInput.command) || '').trim();
    const match = command.match(/task\.py\s+([a-z-]+)/);
    const sub = match && TASK_SUBCOMMANDS[match[1]];
    return sub || null;
  }

  /* trellis CLI 命令：从 command 提取（文档 1.2 节），如 `trellis upgrade`。 */
  function trellisCliActivity(toolInput) {
    const command = String((toolInput && toolInput.command) || '').trim();
    const match = command.match(/^trellis\s+([a-z-]+)/);
    const sub = match && TRELLIS_CLI[match[1]];
    return sub || null;
  }

  function pick(kind, key) {
    const phrases = PHRASES[kind] || PHRASES.generic;
    const next = (phraseCounters.get(key) || 0) + 1;
    phraseCounters.set(key, next);
    return phrases[next % phrases.length];
  }

  function normalizeToolInput(context) {
    const source = context && typeof context === 'object'
      ? (context.toolInput || context.tool_input)
      : null;
    if (!source || typeof source !== 'object') return null;
    const input = {
      filePath: source.filePath || source.file_path || null,
      command: source.command || source.cmd || source.script || null,
      description: source.description || null,
      pattern: source.pattern || null,
      query: source.query || null,
      url: source.url || null,
      prompt: source.prompt || null,
      subagentType: source.subagentType || source.subagent_type || null,
    };
    return Object.values(input).some(Boolean) ? input : null;
  }

  function extensionHint(category, filePath) {
    const path = String(filePath || '').replaceAll('\\', '/').toLowerCase();
    if (!path) return null;
    const isTest = path.includes('tests/') || path.endsWith('tests.swift') || path.endsWith('test.swift');
    const isDoc = path.endsWith('.md') || path.endsWith('.txt') || path.endsWith('.rst');
    const isConfig = path.endsWith('.json') || path.endsWith('.yaml') || path.endsWith('.yml')
      || path.endsWith('.plist') || path.endsWith('.toml');
    if (isTest && category === 'reading') return '检查测试中…';
    if (isTest && category === 'writing') return '优化测试中…';
    if (isDoc && category === 'reading') return '阅读文档中…';
    if (isDoc && category === 'writing') return '更新文档中…';
    if (isConfig && category === 'reading') return '解析配置中…';
    if (isConfig && category === 'writing') return '调整配置中…';
    return null;
  }

  function toolActivity(toolName, toolInput) {
    const tool = String(toolName || '').trim();
    if (!tool) return null;
    const isTrellis = categoryForTool(tool) === 'trellis';
    /* Trellis 工具 / 命令：命令级语义（task.py 子命令、trellis CLI）优先，其次原生工具精确翻译。 */
    if (isTrellis) {
      return taskSubcommandActivity(toolInput)
        || trellisNativeActivity(tool)
        || trellisCliActivity(toolInput)
        || null;
    }
    /* 非 trellis 工具（如 Bash）：若命令本身是 trellis CLI / task.py，仍精确翻译。 */
    if (toolInput && toolInput.command) {
      const cmdActivity = taskSubcommandActivity(toolInput) || trellisCliActivity(toolInput);
      if (cmdActivity) return cmdActivity;
    }
    const category = categoryForTool(tool);
    return extensionHint(category, toolInput && toolInput.filePath)
      || pick(category, `tool:${tool}`);
  }

  function semanticizeActivity(activity, toolName, _action, context) {
    const raw = String(activity || '').trim();
    const tool = String(toolName || '').trim();
    const eventName = String(context && context.eventName || '').trim();
    const normalizedEvent = eventName.toLowerCase();
    const input = normalizeToolInput(context);

    if (normalizedEvent === 'notification') return raw || null;
    if (normalizedEvent === 'userpromptsubmit') return pick('thinking', 'event:UserPromptSubmit');
    if (normalizedEvent === 'pretooluse' || normalizedEvent === 'posttooluse') {
      return toolActivity(tool, input) || raw || null;
    }
    if (normalizedEvent === 'stop' || normalizedEvent === 'sessionend' || normalizedEvent === 'session_end') {
      return pick('done', 'state.done');
    }
    if (normalizedEvent === 'permissionrequest' || normalizedEvent === 'question' || normalizedEvent === 'askuser') {
      return pick('waiting', 'state.waiting');
    }
    if (tool) return toolActivity(tool, input) || raw || null;
    return raw || null;
  }

  return { categoryForTool, extensionHint, semanticizeActivity };
}));
