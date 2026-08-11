/**
 * Trellis Card bridge plugin for OpenCode (opencode.ai).
 *
 * Pure observer: reports OpenCode session/agent activity to Trellis Card over
 * the same socket/inbox channel used by the codex/claude/cursor/pi hooks. It
 * never blocks, rewrites, or injects anything into OpenCode's behavior.
 *
 * Install location: ~/.config/opencode/plugins/trellis-card.js (global scope).
 * OpenCode auto-discovers plugins in this directory for every project, and
 * loads them with Bun (JS runs without a build step).
 *
 * Hooks mapped to Trellis Card HookEvents:
 *   chat.message           -> UserPromptSubmit (activity = user prompt)
 *   tool.execute.before    -> PreToolUse       (activity = tool command)
 *   tool.execute.after     -> PostToolUse      (activity settling)
 *
 * The executable path is resolved lazily so the plugin never throws when
 * Trellis Card is not installed.
 */

import { existsSync } from "node:fs"
import { spawn } from "node:child_process"
import { join } from "node:path"

/* ── Config ──────────────────────────────────────────────────────────── */

function findCardBin() {
  const env = (process.env.TRELLIS_CARD_BIN || "").trim()
  const candidates = [
    env,
    join(process.env.HOME || "", ".local/bin/trellis-card"),
    "/usr/local/bin/trellis-card",
    "/opt/homebrew/bin/trellis-card",
    "trellis-card", // resolve on PATH
  ].filter(Boolean)
  for (const c of candidates) {
    if (c !== "trellis-card") {
      try {
        if (existsSync(c)) return c
      } catch {
        /* fall through */
      }
    }
  }
  return "trellis-card"
}

let cardBin = null
function card() {
  if (cardBin === null) cardBin = findCardBin()
  return cardBin
}

/* ── Session identity ────────────────────────────────────────────────── */

/**
 * OpenCode session key: native sessionID sanitized with an `opencode_`
 * prefix, matching the `pi_<sanitized>` style used by the Pi bridge. Never a
 * process-wide singleton pointer (avoids cross-window contamination).
 */
function resolveSessionKey(input) {
  const gid =
    (input && typeof input.sessionID === "string" && input.sessionID) ||
    process.env.OPENCODE_SESSION_ID ||
    ""
  if (gid) {
    const raw = String(gid)
    const norm = raw.replace(/[^A-Za-z0-9._-]+/g, "_")
    if (!norm) return `opencode_${hashStr(raw)}`
    return `opencode_${norm}${norm === raw ? "" : `_${hashStr(raw)}`}`
  }
  return `opencode_${hashStr(String(Date.now()))}`
}

/* ── Project root ────────────────────────────────────────────────────── */

function findTrellisRoot(start) {
  let cur = String(start || process.cwd() || "")
  for (let i = 0; i < 16 && cur; i++) {
    try {
      if (existsSync(join(cur, ".trellis", "tasks"))) return cur
    } catch {
      return start || ""
    }
    const idx = cur.lastIndexOf("/")
    if (idx <= 0) return start || ""
    cur = cur.slice(0, idx)
  }
  return start || ""
}

/* ── Delivery ────────────────────────────────────────────────────────── */

function send(eventName, payload, input) {
  const bin = card()
  if (!bin) return
  const root = findTrellisRoot(payload.cwd)
  if (!root) return // not a Trellis project -> observe nothing

  const body = {
    ...payload,
    agent: "opencode",
    hook_event_name: eventName,
    session_id: resolveSessionKey(input),
    cwd: root,
    timestamp: Math.floor(Date.now() / 1000),
  }

  try {
    const child = spawn(bin, ["hook", "--agent", "opencode"], {
      stdio: ["pipe", "ignore", "ignore"],
      windowsHide: true,
    })
    child.on("error", () => {
      /* trellis-card not installed -> silent, never disturb OpenCode */
    })
    child.stdin.write(JSON.stringify(body) + "\n")
    child.stdin.end()
    const timer = setTimeout(() => {
      try {
        child.kill()
      } catch {
        /* ignore */
      }
    }, 2000)
    if (timer && typeof timer.unref === "function") timer.unref()
  } catch {
    /* non-fatal: plugin must never break OpenCode */
  }
}

function hashStr(s) {
  let h = 0
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0
  return (h >>> 0).toString(36)
}

/* ── Hooks ───────────────────────────────────────────────────────────── */

/**
 * OpenCode loads plugins as factory functions: `export default async
 * ({ directory, client, $ }) => hooks`. The returned hooks object subscribes
 * to lifecycle events. `directory` is the project root OpenCode is running in.
 */
export default async ({ directory }) => {
  const cwd = directory || process.cwd() || ""

  return {
    "chat.message": async (input, output) => {
      try {
        const parts = output?.parts || []
        const textPart = parts.find((p) => p && p.type === "text")
        const text = (textPart && textPart.text) || ""
        if (!text.trim()) return
        send("UserPromptSubmit", { prompt: text.trim(), cwd }, input)
      } catch (error) {
        /* never break OpenCode */
      }
    },

    "tool.execute.before": async (input, output) => {
      try {
        const tool = (input && input.tool) || ""
        const args = output?.args || {}
        const command =
          typeof args.command === "string"
            ? args.command
            : typeof args === "string"
              ? args
              : ""
        send(
          "PreToolUse",
          { tool_name: tool, command, tool_input: args },
          input,
        )
      } catch (error) {
        /* never break OpenCode */
      }
    },

    "tool.execute.after": async (input, output) => {
      try {
        const tool = (input && input.tool) || ""
        send(
          "PostToolUse",
          { tool_name: tool, is_error: Boolean(output && output.metadata && output.metadata.error) },
          input,
        )
      } catch (error) {
        /* never break OpenCode */
      }
    },
  }
}
