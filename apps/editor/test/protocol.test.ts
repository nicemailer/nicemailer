import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SAVE_PAUSE_MS } from '../src/mailEditor'
import { PROTOCOL_VERSION } from '../src/protocol'
import { documentText, textOffsetToPos } from '../src/text'
import { FakeHost, loadMessage, open, settle } from './helpers'
import { MailEditor } from '../src/mailEditor'

/** Presses a key in the text, as ProseMirror's keymaps read it. `mod`:
 * the command key (Ctrl in jsdom, whose platform is not a Mac). */
function press(editor: MailEditor, key: string, opts: { shift?: boolean; mod?: boolean } = {}): void {
  const event = new KeyboardEvent('keydown', { key, shiftKey: !!opts.shift, ctrlKey: !!opts.mod, bubbles: true, cancelable: true })
  editor.current!.view.dom.dispatchEvent(event)
}

describe('the protocol', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('says it is ready, with its version', () => {
    const { host } = open('text', '')
    expect(host.sent[0]).toEqual({ type: 'ready', v: PROTOCOL_VERSION })
  })

  it('ignores a load of another version', () => {
    document.body.innerHTML = '<div id="editor"></div>'
    const host = new FakeHost()
    const editor = new MailEditor(document.getElementById('editor')!, host)
    host.post({ ...loadMessage('text', 'x'), v: PROTOCOL_VERSION + 1 })
    expect(editor.current).toBeNull()
  })

  it('does not report the content it was loaded with', () => {
    const { host } = open('text', 'hello')
    vi.advanceTimersByTime(SAVE_PAUSE_MS * 2)
    expect(host.count('changed')).toBe(0)
  })

  it('reports a change at the first pause, once', () => {
    const { host, editor } = open('text', 'hello')
    editor.current!.commands.insertContentAt(6, ' you')
    editor.current!.commands.insertContentAt(10, '!')
    vi.advanceTimersByTime(SAVE_PAUSE_MS - 1)
    expect(host.count('changed')).toBe(0)
    vi.advanceTimersByTime(1)
    expect(host.sent.filter(m => m.type === 'changed')).toEqual([{ type: 'changed', content: 'hello you!', mode: 'text', revision: 2 }])
  })

  it('answers a flush at once with the last key typed, and does not report it again', () => {
    const { host, editor } = open('html', '<p>Hi</p>')
    editor.current!.commands.insertContentAt(3, ' there')
    host.post({ type: 'flush', id: 7 })
    expect(host.last('flushed')).toEqual({ type: 'flushed', id: 7, content: '<p>Hi there</p>', mode: 'html', revision: 1 })
    vi.advanceTimersByTime(SAVE_PAUSE_MS * 2)
    expect(host.count('changed')).toBe(0)
  })

  it('switches from text to formatted, quotations made of the quoted lines, and saves', () => {
    const { host, editor } = open('text', 'Thanks\n\n> Lunch?')
    host.post({ type: 'setMode', mode: 'html', content: 'Thanks\n\n> Lunch?' })
    expect(editor.content()).toBe('<p>Thanks</p><p></p><blockquote><p>Lunch?</p></blockquote>')
    expect(host.last('changed')).toMatchObject({ mode: 'html', content: '<p>Thanks</p><p></p><blockquote><p>Lunch?</p></blockquote>' })
  })

  it('switches from formatted to text with the text the host gives', () => {
    const { host, editor } = open('html', '<p><strong>Thanks</strong></p>')
    host.post({ type: 'setMode', mode: 'text', content: 'Thanks\n> q' })
    expect(editor.content()).toBe('Thanks\n> q')
    expect(host.last('changed')).toMatchObject({ mode: 'text', content: 'Thanks\n> q' })
  })

  it('replaces the selection with a text, one line or several', () => {
    const { host, editor } = open('text', 'one two three')
    editor.current!.commands.setTextSelection({ from: 5, to: 8 })
    host.post({ type: 'replaceSelection', text: '2' })
    expect(editor.content()).toBe('one 2 three')
    // Later than the history's grouping delay: a step of its own.
    vi.advanceTimersByTime(1000)
    editor.current!.commands.setTextSelection({ from: 5, to: 6 })
    host.post({ type: 'replaceSelection', text: 'a\nb' })
    expect(editor.content()).toBe('one a\nb three')
    // One step of undo for the whole replacement.
    editor.current!.commands.undo()
    expect(editor.content()).toBe('one 2 three')
  })

  it('replaces the selection with HTML in the formatted mode, through the schema', () => {
    const { host, editor } = open('html', '<p>x</p>')
    editor.current!.commands.setTextSelection({ from: 1, to: 2 })
    host.post({ type: 'replaceSelection', html: '<strong>bold</strong><img src="x">' })
    expect(editor.content()).toBe('<p><strong>bold</strong></p>')
  })

  it('reports what is under the cursor', async () => {
    vi.useRealTimers()
    const { host, editor } = open('html', '<h2><strong>Title</strong></h2><ul><li><p>item</p></li></ul>')
    editor.current!.commands.setTextSelection({ from: 2, to: 4 })
    await settle()
    expect(host.last('state')).toMatchObject({ block: 'h2', empty: false, selection: 'it', marks: { bold: true }, modal: false })
    editor.current!.commands.setTextSelection(10)
    await settle()
    expect(host.last('state')).toMatchObject({ block: 'paragraph', bulletList: true, marks: { bold: false } })
  })
})

describe('the keys', () => {
  it('⇧⇥ outside a list goes back to the subject; in a list, it outdents', () => {
    const { host, editor } = open('html', '<p>text</p><ul><li><p>a</p><ul><li><p>b</p></li></ul></li></ul>')
    editor.current!.commands.setTextSelection(2)
    press(editor, 'Tab', { shift: true })
    expect(host.last('leave')).toEqual({ type: 'leave', direction: 'previous' })
    const leaves = host.count('leave')
    // Inside the nested item "b".
    const pos = editor.current!.state.doc.content.size - 4
    editor.current!.commands.setTextSelection(pos)
    press(editor, 'Tab', { shift: true })
    expect(host.count('leave')).toBe(leaves)
    expect(editor.content()).toBe('<p>text</p><ul><li><p>a</p></li><li><p>b</p></li></ul>')
  })

  it('⇥ indents a list item; outside a list it types a tab in text only, nothing when formatted', () => {
    const { editor } = open('html', '<ul><li><p>a</p></li><li><p>b</p></li></ul><p>x</p>')
    editor.current!.commands.setTextSelection(9)
    press(editor, 'Tab')
    expect(editor.content()).toBe('<ul><li><p>a</p><ul><li><p>b</p></li></ul></li></ul><p>x</p>')
    const end = editor.current!.state.doc.content.size - 1
    editor.current!.commands.setTextSelection(end)
    press(editor, 'Tab')
    expect(editor.content()).toBe('<ul><li><p>a</p><ul><li><p>b</p></li></ul></li></ul><p>x</p>')
    const plain = open('text', 'x').editor
    plain.current!.commands.setTextSelection(2)
    press(plain, 'Tab')
    expect(plain.content()).toBe('x\t')
  })

  it('⌘B, ⌘I and ⌘U format; nothing does in text only', () => {
    const { editor } = open('html', '<p>word</p>')
    editor.current!.commands.setTextSelection({ from: 1, to: 5 })
    press(editor, 'b', { mod: true })
    press(editor, 'i', { mod: true })
    press(editor, 'u', { mod: true })
    expect(editor.content()).toBe('<p><strong><em><u>word</u></em></strong></p>')
    const plain = open('text', 'word').editor
    plain.current!.commands.setTextSelection({ from: 1, to: 5 })
    press(plain, 'b', { mod: true })
    expect(plain.content()).toBe('word')
  })

  it('the typing shortcuts format as one types', () => {
    const { editor } = open('html', '')
    const type = (text: string) => {
      for (const ch of text) {
        const { from, to } = editor.current!.state.selection
        const view = editor.current!.view
        const handled = view.someProp('handleTextInput', f => f(view, from, to, ch, () => view.state.tr.insertText(ch, from, to)))
        if (!handled) view.dispatch(view.state.tr.insertText(ch, from, to))
      }
    }
    type('## ')
    type('Plan')
    expect(editor.content()).toBe('<h2>Plan</h2>')
    const list = open('html', '').editor
    const typeIn = (text: string) => {
      for (const ch of text) {
        const view = list.current!.view
        const { from, to } = view.state.selection
        const handled = view.someProp('handleTextInput', f => f(view, from, to, ch, () => view.state.tr.insertText(ch, from, to)))
        if (!handled) view.dispatch(view.state.tr.insertText(ch, from, to))
      }
    }
    typeIn('- one')
    expect(list.content()).toBe('<ul><li><p>one</p></li></ul>')
  })

  it('⌘K opens the link dialog, which Escape belongs to while it is open', async () => {
    const { host, editor } = open('html', '<p>see doc</p>')
    editor.current!.commands.setTextSelection({ from: 5, to: 8 })
    press(editor, 'k', { mod: true })
    await settle()
    expect(host.last('state')).toMatchObject({ modal: true })
    const url = document.querySelector<HTMLInputElement>('.nm-link-url')!
    url.value = 'exemple.fr/doc'
    url.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    expect(editor.content()).toBe('<p>see <a rel="noopener noreferrer" href="https://exemple.fr/doc">doc</a></p>')
    expect(host.last('state')).toMatchObject({ modal: false })
  })

  it('the link dialog refuses another scheme', async () => {
    const { editor } = open('html', '<p>doc</p>')
    editor.current!.commands.setTextSelection({ from: 1, to: 4 })
    press(editor, 'k', { mod: true })
    const url = document.querySelector<HTMLInputElement>('.nm-link-url')!
    url.value = 'javascript:alert(1)'
    url.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    expect(editor.content()).toBe('<p>doc</p>')
    url.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(document.querySelector<HTMLElement>('.nm-dialog')!.hidden).toBe(true)
  })
})

describe('the toolbar', () => {
  it('is shown in the formatted mode only, with no button to hide it', () => {
    const { host } = open('text', '')
    expect(document.querySelector<HTMLElement>('.nm-toolbar')!.hidden).toBe(true)
    host.post(loadMessage('html', '<p></p>'))
    expect(document.querySelector<HTMLElement>('.nm-toolbar')!.hidden).toBe(false)
    expect(document.querySelector('.nm-toggle')).toBeNull()
    host.post({ type: 'setMode', mode: 'text', content: '' })
    expect(document.querySelector<HTMLElement>('.nm-toolbar')!.hidden).toBe(true)
  })

  it('offers every font, those used recently first, and tells the host', () => {
    const { host, editor } = open('text', '')
    host.post(loadMessage('html', '<p>word</p>', { recentFonts: ['Georgia', 'Gone Font'] }))
    editor.current!.commands.setTextSelection({ from: 1, to: 5 })
    const menu = document.querySelector<HTMLSelectElement>('.nm-font')!
    const groups = [...menu.querySelectorAll('optgroup')].map(g => [g.label, [...g.querySelectorAll('option')].map(o => o.textContent)])
    // A recent font no longer on the Mac is left out.
    expect(groups).toEqual([
      ['font.recent', ['Georgia']],
      ['font.all', ['Arial', 'Avenir Next', 'Georgia', 'Trebuchet MS']],
    ])
    menu.value = "'Avenir Next', sans-serif"
    menu.dispatchEvent(new Event('change'))
    expect(host.last('fontUsed')).toEqual({ type: 'fontUsed', name: 'Avenir Next' })
    const recent = [...menu.querySelector('optgroup')!.querySelectorAll('option')].map(o => o.textContent)
    expect(recent).toEqual(['Avenir Next', 'Georgia'])
    // jsdom writes the name without its quotes, WebKit keeps them.
    expect(editor.content()).toMatch(/font-family: '?Avenir Next'?, sans-serif/)
  })

  it('offers the sizes as numbers', () => {
    open('html', '<p></p>')
    const sizes = [...document.querySelector<HTMLSelectElement>('.nm-size')!.options].map(o => o.textContent)
    expect(sizes).toEqual(['size.normal', '13', '18', '24'])
  })

  it('applies a listed colour, font and size', () => {
    const { editor } = open('html', '<p>word</p>')
    editor.current!.commands.setTextSelection({ from: 1, to: 5 })
    const choose = (name: string, value: string) => {
      const s = document.querySelector<HTMLSelectElement>(`.nm-${name}`)!
      s.value = value
      s.dispatchEvent(new Event('change'))
    }
    choose('color', '#d6312b')
    choose('font', 'Arial, Helvetica, sans-serif')
    choose('size', '24px')
    const html = editor.content()
    expect(html).toMatch(/^<p><span style="[^"]*">word<\/span><\/p>$/)
    expect(html).toMatch(/color: (#d6312b|rgb\(214, 49, 43\))/)
    expect(html).toContain('font-size: 24px')
    expect(html).toMatch(/font-family: Arial/)
    // Back to the default: no style left, no empty span.
    choose('color', '')
    choose('font', '')
    choose('size', '')
    expect(editor.content()).toBe('<p>word</p>')
  })

  it('refuses a value outside the lists', () => {
    const { editor } = open('html', '<p>word</p>')
    editor.current!.commands.setTextSelection({ from: 1, to: 5 })
    expect(editor.current!.commands.setTextColor('#123456')).toBe(false)
    expect(editor.current!.commands.setSize('40px')).toBe(false)
    expect(editor.content()).toBe('<p>word</p>')
  })
})

describe('text offsets in the protocol', () => {
  it('a load puts the cursor at an offset, and the state tells the range', async () => {
    const { host, editor } = open('text', '')
    host.post({ ...loadMessage('text', 'Merci.\n\nOn Monday:\n> Hi'), cursor: 6 })
    await settle()
    expect(host.last('state')?.textRange).toEqual({ start: 6, end: 6 })
    editor.current!.commands.setTextSelection({ from: 1, to: 6 })
    await settle()
    expect(host.last('state')?.textRange).toEqual({ start: 0, end: 5 })
  })

  it('replaces a range of the text in one step of undo', () => {
    const { host, editor } = open('text', 'Merci beaucop.\n> Hi')
    host.post({ type: 'replaceRange', start: 6, end: 13, text: 'beaucoup' })
    expect(editor.content()).toBe('Merci beaucoup.\n> Hi')
    // The cursor after the new words.
    expect(editor.state().textRange).toEqual({ start: 14, end: 14 })
    host.post({ type: 'replaceRange', start: 0, end: 15, text: 'Line one\nLine two.' })
    expect(editor.content()).toBe('Line one\nLine two.\n> Hi')
    editor.current!.commands.undo()
    expect(editor.content()).toBe('Merci beaucop.\n> Hi')
  })

  it('formatted, measures the selection in the text of its blocks', async () => {
    // The text the core measures a passage in (`AiView::view`): a line per
    // paragraph, heading or list item, a ⇧↩ a line break too.
    const { editor } = open('html', '<h2>Plan</h2><ul><li><p>un</p></li></ul><p>Je <strong>viens</strong><br>jeudi</p>')
    expect(documentText(editor.current!.state.doc)).toBe('Plan\nun\nJe viens\njeudi')
    const doc = editor.current!.state.doc
    const from = textOffsetToPos(doc, 11)
    const to = textOffsetToPos(doc, 16)
    expect(doc.textBetween(from, to)).toBe('viens')
    editor.current!.commands.setTextSelection({ from, to })
    expect(editor.state().textRange).toEqual({ start: 11, end: 16 })
  })

  it('formatted, a passage replaced keeps its formatting, in one step of undo', () => {
    const { host, editor } = open('html', '<h2>Plan</h2><p>Je <strong>viens</strong> jeudi.</p>')
    host.post({ type: 'replaceRange', start: 8, end: 13, text: 'arrive' })
    expect(editor.content()).toBe('<h2>Plan</h2><p>Je <strong>arrive</strong> jeudi.</p>')
    editor.current!.commands.undo()
    expect(editor.content()).toBe('<h2>Plan</h2><p>Je <strong>viens</strong> jeudi.</p>')
  })

  it('formatted, the whole HTML is replaced in one step of undo', () => {
    vi.useFakeTimers()
    const { host, editor } = open('html', '<p>oui</p>')
    host.post({ type: 'replaceContent', html: '<p>Oui, <em>merci</em>.</p>' })
    expect(editor.content()).toBe('<p>Oui, <em>merci</em>.</p>')
    vi.advanceTimersByTime(SAVE_PAUSE_MS * 2)
    expect(host.last('changed')?.content).toBe('<p>Oui, <em>merci</em>.</p>')
    editor.current!.commands.undo()
    expect(editor.content()).toBe('<p>oui</p>')
    vi.useRealTimers()
  })

  it('a formatted load can convert a text, without reporting a change', () => {
    vi.useFakeTimers()
    const { host, editor } = open('text', '')
    host.post({ ...loadMessage('html', 'Hello\n> quoted'), fromText: true })
    expect(editor.content()).toBe('<p>Hello</p><blockquote><p>quoted</p></blockquote>')
    vi.advanceTimersByTime(SAVE_PAUSE_MS * 2)
    expect(host.count('changed')).toBe(0)
    vi.useRealTimers()
  })
})
