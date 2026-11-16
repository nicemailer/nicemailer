import type { Formats } from '../src/formats'
import { MailEditor } from '../src/mailEditor'
import type { EditorMessage, HostMessage, LoadMessage, Mode } from '../src/protocol'
import { PROTOCOL_VERSION } from '../src/protocol'
import type { Transport } from '../src/transport'

/** The lists the app gives: the fonts of the Mac (those of
 * `nm_core::formats::FONTS` with their stacks, the others with their
 * generic family), the sizes and colours of `compose_formats`. */
export const formats: Formats = {
  fonts: [
    { name: 'Arial', stack: 'Arial, Helvetica, sans-serif' },
    { name: 'Avenir Next', stack: "'Avenir Next', sans-serif" },
    { name: 'Georgia', stack: "Georgia, 'Times New Roman', serif" },
    { name: 'Trebuchet MS', stack: "'Trebuchet MS', Helvetica, sans-serif" },
  ],
  sizes: [
    { key: '13', css: '13px' },
    { key: '18', css: '18px' },
    { key: '24', css: '24px' },
  ],
  colors: [
    { key: 'red', hex: '#d6312b' },
    { key: 'blue', hex: '#2f6fd1' },
  ],
}

/** A transport that records what the editor posts. */
export class FakeHost implements Transport {
  sent: EditorMessage[] = []
  private handler: ((m: HostMessage) => void) | null = null
  send(message: EditorMessage): void {
    this.sent.push(message)
  }
  listen(handler: (m: HostMessage) => void): void {
    this.handler = handler
  }
  post(message: HostMessage): void {
    this.handler?.(message)
  }
  last<T extends EditorMessage['type']>(type: T): Extract<EditorMessage, { type: T }> | undefined {
    return [...this.sent].reverse().find(m => m.type === type) as Extract<EditorMessage, { type: T }> | undefined
  }
  count(type: EditorMessage['type']): number {
    return this.sent.filter(m => m.type === type).length
  }
}

export function loadMessage(mode: Mode, content: string, extra: Partial<LoadMessage> = {}): LoadMessage {
  return {
    type: 'load',
    v: PROTOCOL_VERSION,
    mode,
    content,
    theme: {
      dark: false,
      font: 'Menlo',
      size: 14,
      text: '#000',
      background: '#fff',
      muted: '#888',
      accent: '#06c',
      selection: '#cde',
      border: '#ccc',
      panel: '#eee',
    },
    strings: { 'font.default': 'Default' },
    formats,
    recentFonts: [],
    focus: false,
    ...extra,
  }
}

/** An editor on a fresh page, loaded with `content` in `mode`. */
export function open(mode: Mode, content: string): { host: FakeHost; editor: MailEditor } {
  document.body.innerHTML = '<div id="editor"></div>'
  const host = new FakeHost()
  const editor = new MailEditor(document.getElementById('editor')!, host)
  host.post(loadMessage(mode, content))
  return { host, editor }
}

/** Lets the microtasks run (the `state` message is posted in one). */
export async function settle(): Promise<void> {
  await new Promise(r => setTimeout(r, 0))
}
