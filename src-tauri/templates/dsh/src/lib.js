/**
 * dsh-trellis-bridge core logic — pure mapping from dsh session/event to
 * Trellis Card hook payloads. Split from the cordis plugin entry so the
 * mapping is unit-testable without a running dsh.
 *
 * `emit` is the only entry point: given a session + a session/event, it
 * produces zero or more hook payloads delivered through the injectable
 * `deliver` callback. The plugin entry wires `deliver` to spawn the
 * trellis-card binary.
 */

import { existsSync } from 'node:fs'
import { join } from 'node:path'

export function findTrellisRoot(start) {
  let cur = start || ''
  for (let i = 0; i < 16 && cur; i++) {
    try {
      if (existsSync(join(cur, '.trellis', 'tasks'))) return cur
    } catch {
      return ''
    }
    const parent = cur.slice(0, Math.max(cur.lastIndexOf('/'), cur.lastIndexOf('\\')))
    if (parent === cur || parent === '') return ''
    cur = parent
  }
  return ''
}

function blockText(block) {
  if (!block || typeof block !== 'object') return ''
  switch (block.type) {
    case 'text':
      return typeof block.text === 'string' ? block.text : ''
    case 'reasoning':
      return typeof block.text === 'string' ? block.text : ''
    case 'tool-call':
      return block.name || ''
    case 'tool-result': {
      const inner = Array.isArray(block.content)
        ? block.content.map((b) => blockText(b)).filter(Boolean).join(' ')
        : ''
      return inner
    }
    default:
      return ''
  }
}

/** Join all text blocks of a message content array into one line. */
export function contentText(content) {
  if (!Array.isArray(content)) return ''
  const parts = content.map((b) => blockText(b)).filter((t) => t.length > 0)
  const joined = parts.join(' ')
  return joined.length > 4000 ? joined.slice(0, 4000) : joined
}

function hashStr(s) {
  let h = 0
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0
  return (h >>> 0).toString(36)
}

/**
 * Translate one dsh session/event into Trellis Card hook payloads.
 * @param session - the dsh Session object (needs `.id` and `.header.cwd`).
 * @param event - the SessionEvent ({type, time, data}).
 * @param deliver - (payload) => void, called once per hook event to send.
 * @param seen - optional Set of session ids that already fired SessionStart.
 */
export function emit(session, event, deliver, seen = new Set()) {
  const cwd = (session.header && session.header.cwd) || ''
  const project = findTrellisRoot(cwd)
  if (!project) return // not a Trellis project -> observe nothing

  const sessionId = String(session.id || '')
  const data = event.data || {}
  const base = {
    session_id: sessionId || `dsh_${hashStr(project + sessionId)}`,
    cwd: project,
  }

  switch (event.type) {
    case 'user/message': {
      // A human prompt (or injected context) entered the surface.
      const text = contentText(data.content)
      if (!text) return
      if (!seen.has(sessionId)) {
        seen.add(sessionId)
        deliver({ ...base, hook_event_name: 'SessionStart' })
      }
      deliver({ ...base, hook_event_name: 'UserPromptSubmit', prompt: text })
      break
    }
    case 'tool/call': {
      const name = data.name || ''
      if (!name) return
      let toolInput = {}
      try {
        const parsed = JSON.parse(data.arguments || '{}')
        if (parsed && typeof parsed === 'object') toolInput = parsed
      } catch {
        toolInput = { text: data.arguments || '' }
      }
      deliver({
        ...base,
        hook_event_name: 'PreToolUse',
        tool_name: name,
        tool_input: toolInput,
        command: typeof toolInput.command === 'string' ? toolInput.command : '',
      })
      break
    }
    case 'tool/result': {
      // ToolResultBlock carries toolCallId, not the tool name; report settling.
      const blocks = data.message && data.message.content
      const toolCallId = Array.isArray(blocks) && blocks[0] && blocks[0].toolCallId
        ? String(blocks[0].toolCallId)
        : ''
      deliver({
        ...base,
        hook_event_name: 'PostToolUse',
        tool_name: '',
        tool_call_id: toolCallId,
        is_error: Boolean(data.error),
      })
      break
    }
    case 'step/start': {
      if (!seen.has(sessionId)) {
        seen.add(sessionId)
        deliver({ ...base, hook_event_name: 'SessionStart' })
      } else {
        deliver({ ...base, hook_event_name: 'StepStart', activity: `step ${data.turn}.${data.step}` })
      }
      break
    }
    case 'turn/end': {
      deliver({ ...base, hook_event_name: 'Stop', reason: data.reason && data.reason.kind })
      break
    }
    default:
      break
  }
}
