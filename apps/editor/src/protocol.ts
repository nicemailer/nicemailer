// The messages between the editor and its host (`specifications/edition.md`,
// section 8; the reference for any host is `PROTOCOL.md`).
//
// One channel of JSON messages, each way. Nothing here assumes WebKit: the
// app (a `WKWebView`) and a web client (an `<iframe>`) are both hosts.
//
// TypeScript note: each message is an object with a `type` field, and the
// union of their interfaces is a "discriminated union": after
// `if (msg.type === 'load')`, TypeScript knows `msg` is a `LoadMessage`.

import type { Formats } from './formats'

/** Version of the protocol; a host refuses an editor of another one. */
export const PROTOCOL_VERSION = 1

/** `text`: text only; `html`: formatted. */
export type Mode = 'text' | 'html'

/** Colours, font and size of the host's theme, as CSS values. */
export interface Theme {
  dark: boolean
  /** Font of the text in text only, and of the editor's own interface. */
  font: string
  /** Size of the text, in pixels (the host's text size, `⌘+` `⌘−`). */
  size: number
  text: string
  background: string
  muted: string
  accent: string
  selection: string
  border: string
  /** Background of the formatting bar and of the dialogs. */
  panel: string
}

/** The texts of the editor's interface, in the host's language. */
export type Strings = Record<string, string>

// ---- Host to editor ----

export interface LoadMessage {
  type: 'load'
  v: number
  mode: Mode
  /** The text (text only) or the HTML (formatted) of the draft. */
  content: string
  theme: Theme
  strings: Strings
  formats: Formats
  /** The fonts chosen last, most recent first (names of `formats.fonts`):
   * the top of the font menu. The host remembers them (`fontUsed`). */
  recentFonts: string[]
  /** Give the cursor to the editor at once. */
  focus: boolean
  /** Formatted mode only: `content` is a text, converted on load as
   * `setMode` would (a proposal the core wrote as text, opened in the
   * formatted mode of `compose.format`). No `changed` follows. */
  fromText?: boolean
  /** Text only: where the cursor goes, in UTF-16 units of the text
   * (default 0, the start: a reply is written above the quotation). */
  cursor?: number
}

export interface ThemeMessage {
  type: 'theme'
  theme: Theme
}

export interface SetModeMessage {
  type: 'setMode'
  mode: Mode
  /** The content in the new mode: the text for both (the host asked the
   * core for the text of an HTML; the editor converts a text to HTML). */
  content: string
}

export interface ReplaceSelectionMessage {
  type: 'replaceSelection'
  /** Text: inserted as paragraphs, one per line. */
  text?: string
  /** HTML: read through the schema, as a paste (formatted mode). */
  html?: string
}

/** Replaces the range `start`..`end` of the document's text (UTF-16 units
 * of `documentText`: the text in text only, a line per text block
 * formatted) with `text`, in one step of undo: an AI change the core
 * applied (text only), a passage the AI rewrote (formatted, where the
 * text keeps the formatting of the passage). */
export interface ReplaceRangeMessage {
  type: 'replaceRange'
  start: number
  end: number
  text: string
}

/** Formatted mode: the whole HTML replaced, in one step of undo (the AI
 * rewrote the user's blocks, `ai_apply`). */
export interface ReplaceContentMessage {
  type: 'replaceContent'
  html: string
}

export interface FlushMessage {
  type: 'flush'
  id: number
}

export interface FocusMessage {
  type: 'focus'
}

/** The keys went to another field: the editor gives up the focus. */
export interface BlurMessage {
  type: 'blur'
}

export type HostMessage =
  | LoadMessage
  | ThemeMessage
  | SetModeMessage
  | ReplaceSelectionMessage
  | ReplaceRangeMessage
  | ReplaceContentMessage
  | FlushMessage
  | FocusMessage
  | BlurMessage

// ---- Editor to host ----

export interface ReadyMessage {
  type: 'ready'
  v: number
}

export interface ChangedMessage {
  type: 'changed'
  /** The text (text only) or the HTML (formatted). */
  content: string
  mode: Mode
  /** Increases with each change, from 0 at each `load`. */
  revision: number
}

export interface FlushedMessage {
  type: 'flushed'
  id: number
  content: string
  mode: Mode
  revision: number
}

/** What is under the cursor, for the bar and for the host's keys. */
export interface EditorState {
  type: 'state'
  /** A dialog of the editor is open (link, a menu of the bar): Escape is its. */
  modal: boolean
  /** The text has the focus (a click in it gives it the keys). */
  focused: boolean
  /** The selection in UTF-16 units of the document's text
   * (`documentText`: the text in text only, a line per text block
   * formatted), the cursor when `start == end` (the AI's passage, the UI
   * scenarios). */
  textRange: { start: number; end: number } | null
  /** Nothing is selected. */
  empty: boolean
  /** The text selected (for the AI palette), at most 20,000 characters. */
  selection: string
  marks: {
    bold: boolean
    italic: boolean
    underline: boolean
    link: boolean
  }
  /** `paragraph`, `h1`, `h2`, `h3`. */
  block: string
  bulletList: boolean
  orderedList: boolean
  blockquote: boolean
  color: string | null
  font: string | null
  size: string | null
}

export interface LeaveMessage {
  type: 'leave'
  /** `previous`: `⇧⇥` outside a list, the keys go back to the subject. */
  direction: 'previous'
}

/** A font was chosen in the bar's menu: the host puts it first among the
 * fonts used recently, for the next drafts. */
export interface FontUsedMessage {
  type: 'fontUsed'
  name: string
}

export type EditorMessage =
  | ReadyMessage
  | ChangedMessage
  | FlushedMessage
  | EditorState
  | LeaveMessage
  | FontUsedMessage
