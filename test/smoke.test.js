'use strict';

const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const assert = require('node:assert/strict');

const projectRoot = path.resolve(__dirname, '..');
const read = (file) => fs.readFileSync(path.join(projectRoot, file), 'utf8');

test('frontend keeps local assets and the core observation surfaces', () => {
  const html = read('src/index.html');
  const app = read('src/app.js');
  const styles = read('src/styles.css');
  const themes = read('src/theme-variants.css');

  for (const id of [
    'capsule',
    'capPreview',
    'capMenuPop',
    'capProject',
    'capPreviewProject',
    'card',
    'flip',
    'back',
    'btnCapsule',
    'btnFlip',
  ]) {
    assert.match(html, new RegExp(`id="${id}"`));
  }

  for (const asset of ['styles.css', 'layout-variants.css', 'theme-variants.css', 'vendor/gsap.min.js', 'focus-policy.js', 'activity-display.js', 'app.js']) {
    assert.match(html, new RegExp(`(?:src|href)="${asset.replaceAll('.', '\\.')}`));
  }

  assert.doesNotMatch(html, /fonts\.(googleapis|gstatic)\.com/);
  assert.match(app, /function setMode\(mode\)/);
  assert.match(app, /function renderCapsule\(t, projectActivity\)/);
  assert.match(app, /const tool = String\(toolName \|\| ''\)\.trim\(\)/);
  assert.match(app, /t\.startsWith\('trellis-'\) \|\| t\.startsWith\('trellis:'\)/);
  assert.match(app, /formatter\.semanticizeActivity\(raw, tool, action, context\) \|\| tool/);
  assert.match(app, /const semanticActivity = displayActivity\(/);
  assert.match(app, /runtimeActivityHtml\(rawActivity \|\| '最近没有 Agent 活动'\)/);
  assert.match(app, /const capsuleActivity = rawActivity \|\| semanticActivity/);
  assert.match(app, /function holdArchivedFocus\(focusKey, runtimeView\)/);
  assert.match(app, /data-next-after-archive/);
  assert.match(app, /action === 'archive'/);
  assert.match(styles, /\.archive-receipt/);
  assert.match(app, /\.tree-archive/);
  assert.match(app, /archiveTreeTask\(task, archBtn\)/);
  assert.match(app, /call\('archive_task', \{ project: task\.projectPath \|\| task\.project, task: task\.dir \|\| task\.id \}\)/);
  assert.match(app, />归档<\/button>/);
  assert.match(styles, /\.tree-archive\{/);
  assert.match(styles, /\.runtime-activity[\s\S]*-webkit-line-clamp:2/);
  assert.match(app, /function renderBack\(t, docs, loading, error\)/);
  assert.match(app, /function mdRender\(src\)/);
  assert.match(app, /class="doc-section"/);
  assert.match(app, /progressTitle/);
  assert.match(app, /function metricTags\(t\)/);
  assert.match(app, /detailMetrics/);
  assert.match(html, /id="themePop"/);
  assert.match(html, /id="btnTheme"/);
  assert.match(app, /function applyTheme\(theme, persist = true\)/);
  assert.match(app, /function renderThemePicker\(\)/);
  const themeIds = new Set([...app.matchAll(/id: '([^']+)'/g)].map((match) => match[1]));
  assert.equal([...themeIds].filter((id) => themes.includes(`body[data-theme="${id}"]`)).length, 20);
  const cssThemeIds = new Set([...themes.matchAll(/body\[data-theme="([^"]+)"\]/g)].map((match) => match[1]));
  assert.equal(cssThemeIds.size, 20);
});

test('Tauri content policy is local and permits only its IPC channel', () => {
  const config = JSON.parse(read('src-tauri/tauri.conf.json'));
  const csp = config.app && config.app.security && config.app.security.csp;

  assert.equal(typeof csp, 'string');
  assert.match(csp, /default-src 'self'/);
  assert.match(csp, /connect-src ipc: http:\/\/ipc\.localhost/);
  assert.match(csp, /script-src 'self'/);
  assert.match(csp, /font-src 'self'/);
  assert.match(csp, /object-src 'none'/);
});

test('settings expose guided Hook management through Tauri IPC', () => {
  const app = read('src/app.js');
  const styles = read('src/styles.css');

  assert.match(app, /function refreshHookStatuses\(force = false\)/);
  assert.match(app, /function toggleHook\(agent\)/);
  assert.match(app, /call\('get_hook_statuses'\)/);
  assert.match(app, /call\('configure_hook', \{ agent, enabled \}\)/);
  assert.match(app, /让 Trellis Card 自动收到活动/);
  assert.match(app, /本机全局接入/);
  assert.match(app, /重启对应 Agent/);
  assert.match(styles, /\.hook-guide/);
  assert.match(styles, /\.hook-state-installed/);
});

test('first-run setup offers existing-project scan or Hook-driven empty start', () => {
  const html = read('src/index.html');
  const app = read('src/app.js');

  assert.match(html, /id="btnScanExisting"/);
  assert.match(html, /扫描已有项目/);
  assert.match(html, /选择一个根目录/);
  assert.match(html, /id="btnStartEmpty"/);
  assert.match(html, /从零开始/);
  assert.match(html, /安装 Hook 后，Agent 运行 Trellis 项目时会自动导入/);
  assert.match(app, /async function startWithoutProject\(\)/);
  assert.match(app, /call\('complete_setup'\)/);
  assert.match(app, /async function enterMain\(\{ openHookSetup = false \} = \{\}\)/);
  assert.match(app, /enterMain\(\{ openHookSetup: true \}\)/);
  assert.match(app, /if \(openHookSetup && !state\.adminOpen\) toggleAdmin\(\)/);
  assert.match(app, /等待项目接入/);
});
