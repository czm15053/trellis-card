/**
 * dsh-trellis-bridge — Trellis Card bridge plugin for DSH (DeepSeek Harness).
 *
 * Pure observer: forwards dsh session/tool activity to Trellis Card over the
 * same socket/inbox channel used by the codex/claude/cursor/pi/opencode hooks.
 * It never blocks, rewrites, or injects anything into dsh's behavior.
 *
 * Install: `dsh plugin --profile web add link:<this-dir>` (the
 * dsh.bundle.patch manifest field points at ./cordis.patch.yml). Runs in the
 * dsh host process, subscribing to the `session/event` firehose.
 *
 * The mapping logic lives in ./lib.js (unit-testable); this file is the
 * cordis plugin entry that wires it to deliver to Trellis Card.
 *
 * Delivery: spawn the trellis-card binary per event (same as Pi/OpenCode
 * bridges). Trellis Card's socket server reads the whole stream until EOF
 * (read_to_string), so the connection must close per event — a persistent
 * socket never flushes. Spawn per event guarantees delivery.
 */

import { existsSync } from 'node:fs'
import { spawn } from 'node:child_process'
import { join } from 'node:path'
import { emit } from './lib.js'

export const name = 'dsh-trellis-bridge'

/** No service injection needed: `session/event` rides the cordis event bus. */
export const inject = []

/* ── Trellis Card binary discovery ────────────────────────────────────── */

function findCardBin() {
  const env = (process.env.TRELLIS_CARD_BIN || '').trim()
  const home = process.env.HOME || process.env.USERPROFILE || ''
  const isWin = process.platform === 'win32'
  const candidates = [
    env,
    ...(isWin
      ? [
          join(process.env.LOCALAPPDATA || home, 'Programs', 'Trellis-Card', 'trellis-card.exe'),
          join(home, '.local', 'bin', 'trellis-card.exe'),
        ]
      : [
          join(home, '.local', 'bin', 'trellis-card'),
          '/usr/local/bin/trellis-card',
          '/opt/homebrew/bin/trellis-card',
        ]),
    'trellis-card',
  ].filter(Boolean)
  for (const c of candidates) {
    if (c !== 'trellis-card') {
      try {
        if (existsSync(c)) return c
      } catch {
        /* fall through */
      }
    }
  }
  return 'trellis-card'
}

let cardBin = null
function card() {
  if (cardBin === null) cardBin = findCardBin()
  return cardBin
}

/** Spawn trellis-card hook --agent dsh and pipe one JSON payload to stdin. */
function deliver(payload) {
  const bin = card()
  if (!bin) return
  const body = {
    ...payload,
    agent: 'dsh',
    timestamp: Math.floor(Date.now() / 1000),
  }
  try {
    const child = spawn(bin, ['hook', '--agent', 'dsh'], {
      stdio: ['pipe', 'ignore', 'ignore'],
      windowsHide: true,
    })
    child.on('error', () => {
      /* trellis-card not installed -> silent, never disturb dsh */
    })
    child.stdin.write(JSON.stringify(body) + '\n')
    child.stdin.end()
    const timer = setTimeout(() => {
      try {
        child.kill()
      } catch {
        /* ignore */
      }
    }, 2000)
    if (timer && typeof timer.unref === 'function') timer.unref()
  } catch {
    /* non-fatal: the bridge must never break dsh */
  }
}

/** Per-plugin delivery function (injected into emit). */
const deliverHook = (payload) => deliver(payload)

/** Track whether each session already sent a SessionStart. */
const seen = new Set()

/** Per-plugin cache: project roots and tool names keyed by session id. */
const cache = { projectBySession: new Map(), toolNameBySession: new Map() }

/* ── Plugin entry ─────────────────────────────────────────────────────── */

export function apply(ctx) {
  // The subscription is an effect on the plugin's fiber: cordis disposes it
  // when the plugin unloads, so no manual cleanup is needed. Subscribe on the
  // root scope — session events are emitted from the session store which may
  // live on a sibling/root scope, so a plugin-local `ctx.on` would miss them.
  ctx.effect(() => {
    const dispose = ctx.root.on('session/event', (session, event) => {
      try {
        emit(session, event, deliverHook, seen, cache)
      } catch {
        /* observer must never break dsh */
      }
    })
    return dispose
  }, 'dsh-trellis-bridge.listeners')
}
