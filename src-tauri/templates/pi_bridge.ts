/**
 * Trellis Card bridge extension for Pi (pi-coding-agent).
 *
 * Pure observer: reports Pi session/agent activity to Trellis Card over the
 * same socket/inbox channel used by the codex/claude/cursor hooks. It never
 * blocks, rewrites, or injects anything into Pi's behavior.
 *
 * Install location: ~/.pi/agent/extensions/trellis-card.ts (global scope).
 * Pi auto-discovers extensions in this directory for every project. Pi loads
 * extensions with jiti, so TypeScript runs without a build step.
 *
 * Events mapped to Trellis Card HookEvents:
 *   session_start          -> SessionStart
 *   before_agent_start     -> UserPromptSubmit  (activity = user prompt)
 *   tool_call              -> PreToolUse        (activity = tool input)
 *   tool_execution_end     -> PostToolUse       (activity settling)
 *
 * The executable path is resolved lazily so the extension never throws when
 * Trellis Card is not installed.
 */

import { existsSync } from "node:fs";
import { spawn } from "node:child_process";
import { dirname, join } from "node:path";

/* ── Config ───────────────────────────────────────────────────────────── */

function findCardBin(): string {
  const env = (process.env.TRELLIS_CARD_BIN || "").trim();
  /* Windows 用 USERPROFILE，macOS/Linux 用 HOME。候选路径按平台区分：
     Windows 走 %LOCALAPPDATA% 与 PATH；Unix 走常见安装路径与 PATH。 */
  const home = process.env.HOME || process.env.USERPROFILE || "";
  const isWin = process.platform === "win32";
  const candidates = [
    env,
    ...(isWin
      ? [
          join(process.env.LOCALAPPDATA || home, "Programs", "Trellis-Card", "trellis-card.exe"),
          join(home, ".local", "bin", "trellis-card.exe"),
        ]
      : [
          join(home, ".local", "bin", "trellis-card"),
          "/usr/local/bin/trellis-card",
          "/opt/homebrew/bin/trellis-card",
        ]),
    "trellis-card", // resolve on PATH
  ].filter(Boolean) as string[];
  for (const c of candidates) {
    if (c !== "trellis-card") {
      try {
        if (existsSync(c)) return c;
      } catch {
        /* fall through */
      }
    }
  }
  return "trellis-card";
}

let cardBin: string | null = null;
function card(): string | null {
  if (cardBin === null) cardBin = findCardBin();
  return cardBin;
}

/* ── Session identity ─────────────────────────────────────────────────── */

/**
 * Authoritative Pi session key, following upstream Trellis contextKey():
 * native session id first, then env, then session file path. Never a
 * process-wide singleton pointer (prevents cross-window contamination).
 */
function resolveSessionKey(ctx: any): string {
  const gid =
    ctx?.sessionManager?.getSessionId?.() ||
    process.env.PI_SESSION_ID ||
    process.env.PI_SESSIONID ||
    "";
  if (gid) {
    const raw = String(gid);
    const norm = raw.replace(/[^A-Za-z0-9._-]+/g, "_");
    if (!norm) return `pi_${hashStr(raw)}`;
    return `pi_${norm}${norm === raw ? "" : `_${hashStr(raw)}`}`;
  }
  const file =
    ctx?.sessionManager?.getSessionFile?.() || process.env.PI_SESSION_FILE || "";
  if (file) return `pi_transcript_${hashStr(String(file))}`;
  return `pi_${hashStr(String(Date.now()))}`;
}

/* ── Project root ─────────────────────────────────────────────────────── */

function resolveProjectRoot(ctx: any): string {
  const cwd =
    ctx?.sessionManager?.getCwd?.() ||
    process.cwd() ||
    "";
  return findTrellisRoot(String(cwd));
}

/** Walk up from cwd looking for a `.trellis/tasks` directory. */
function findTrellisRoot(start: string): string {
  let cur = start || process.cwd() || "";
  for (let i = 0; i < 16 && cur; i++) {
    try {
      if (existsSync(join(cur, ".trellis", "tasks"))) return cur;
    } catch {
      return start;
    }
    const parent = dirname(cur);
    if (parent === cur) return start; /* 到根目录，停止 */
    cur = parent;
  }
  return start;
}

/* ── Delivery ─────────────────────────────────────────────────────────── */

function send(eventName: string, payload: Record<string, unknown>, ctx: any) {
  const bin = card();
  if (!bin) return;
  const root = resolveProjectRoot(ctx);
  if (!root) return; // not a Trellis project -> observe nothing

  const body: Record<string, unknown> = {
    ...payload,
    agent: "pi",
    hook_event_name: eventName,
    session_id: resolveSessionKey(ctx),
    cwd: root,
    timestamp: Math.floor(Date.now() / 1000),
  };

  try {
    const child = spawn(bin, ["hook", "--agent", "pi"], {
      stdio: ["pipe", "ignore", "ignore"],
      windowsHide: true,
    });
    child.on("error", () => {
      /* trellis-card not installed -> silent, never disturb Pi */
    });
    child.stdin.write(JSON.stringify(body) + "\n");
    child.stdin.end();
    const timer: any = setTimeout(() => {
      try {
        child.kill();
      } catch {
        /* ignore */
      }
    }, 2000);
    if (timer && typeof timer.unref === "function") timer.unref();
  } catch {
    /* non-fatal: extension must never break Pi */
  }
}

function hashStr(s: string): string {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return (h >>> 0).toString(36);
}

/* ── Events ───────────────────────────────────────────────────────────── */

export default function (pi: any) {
  pi.on?.("session_start", (event: any, ctx: any) => {
    send("SessionStart", { reason: event?.reason || "startup" }, ctx);
  });

  pi.on?.("before_agent_start", (event: any, ctx: any) => {
    const prompt =
      typeof event?.prompt === "string" && event.prompt.trim()
        ? event.prompt.trim()
        : undefined;
    if (!prompt) return;
    send("UserPromptSubmit", { prompt }, ctx);
  });

  pi.on?.("tool_call", (event: any, ctx: any) => {
    const toolName =
      (typeof event?.toolName === "string" && event.toolName) ||
      (typeof event?.tool_name === "string" && event.tool_name) ||
      "";
    const input = event?.input ?? event?.toolInput ?? {};
    const command =
      (typeof input === "object" &&
        input !== null &&
        typeof input.command === "string"
        ? input.command
        : "") ||
      (typeof input === "string" ? input : "") ||
      "";
    send(
      "PreToolUse",
      {
        tool_name: toolName,
        tool_input:
          typeof input === "object" && input !== null
            ? input
            : { text: String(input) },
        command,
      },
      ctx,
    );
  });

  // Observe tool completion so the card can mark activity settling.
  pi.on?.("tool_execution_end", (event: any, ctx: any) => {
    const toolName =
      (typeof event?.toolName === "string" && event.toolName) || "";
    send("PostToolUse", { tool_name: toolName, is_error: Boolean(event?.isError) }, ctx);
  });
}
