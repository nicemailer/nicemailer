import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { toHex } from '../src/formats'
import { open } from './helpers'

const fixture = (name: string) => readFileSync(join(dirname(fileURLToPath(import.meta.url)), 'fixtures', name), 'utf8')

/**
 * The HTML with each `style` written one way: jsdom spells a colour
 * `rgb(…)` and drops the quotes of a font name where WebKit keeps what was
 * set; the core rewrites both anyway (`mail_html::span_style`).
 */
function canonical(html: string): string {
  return html.replace(/ style="([^"]*)"/g, (_, style: string) => {
    const decls = style
      .split(';')
      .map(d => d.trim())
      .filter(Boolean)
      .map(d => {
        const [name = '', ...rest] = d.split(':')
        let value = rest.join(':').trim()
        if (name.trim() === 'color') value = toHex(value) ?? value
        if (name.trim() === 'font-family') value = (value.split(',')[0] ?? '').replace(/['"]/g, '').trim()
        return `${name.trim()}: ${value}`
      })
    return ` style="${decls.join('; ')}"`
  })
}

/** The HTML of a formatted editor loaded with `html`. */
function through(html: string): string {
  return canonical(open('html', html).editor.content())
}

/** The HTML after `html` is pasted into an empty formatted editor. */
function pasted(html: string): string {
  const { editor } = open('html', '')
  editor.current!.view.pasteHTML(html)
  return canonical(editor.content())
}

describe('the formatted schema', () => {
  it('keeps what it knows', () => {
    const html =
      '<h1>T</h1><h2>U</h2><h3>V</h3><p><strong>b</strong> <em>i</em> <u>u</u> <a rel="noopener noreferrer" href="https://x.fr">l</a></p>' +
      '<ul><li><p>one</p><ul><li><p>deep</p></li></ul></li></ul><ol start="3"><li><p>three</p></li></ol>' +
      '<blockquote><p>q</p></blockquote><p>a<br>b</p>'
    expect(through(html)).toBe(html)
  })

  it('drops what it does not know, the text kept', () => {
    const out = through('<h4>four</h4><table><tr><td>cell</td></tr></table><p><s>strike</s> <code>code</code></p><hr><img src="x.png">')
    expect(out).not.toMatch(/<(h4|table|td|s|code|hr|img)\b/)
    for (const text of ['four', 'cell', 'strike', 'code']) expect(out).toContain(text)
  })

  it('keeps only listed colours, fonts and sizes, normalized', () => {
    expect(through('<p><span style="color: rgb(47, 111, 209); font-family: &quot;Georgia&quot;, serif; font-size: 18px">x</span></p>')).toBe(
      '<p><span style="color: #2f6fd1; font-family: Georgia; font-size: 18px">x</span></p>',
    )
    expect(through('<p><span style="color: #123456; font-family: Comic Sans MS; font-size: 40px">x</span></p>')).toBe('<p>x</p>')
  })

  it('keeps only safe links', () => {
    expect(through('<p><a href="javascript:alert(1)">x</a> <a href="file:///etc">y</a></p>')).toBe('<p>x y</p>')
    expect(through('<p><a href="mailto:a@x.fr">a</a></p>')).toBe('<p><a rel="noopener noreferrer" href="mailto:a@x.fr">a</a></p>')
  })

  it('gives back its own HTML unchanged (a draft reopened)', () => {
    const first = through(fixture('obsidian.html'))
    expect(through(first)).toBe(first)
    const second = through(fixture('safari.html'))
    expect(through(second)).toBe(second)
  })
})

describe('pasting', () => {
  it('from Obsidian keeps headings, lists, quotation and links, not the rest', () => {
    const out = pasted(fixture('obsidian.html'))
    expect(out).toContain('<h1>Plan de la semaine</h1>')
    expect(out).toContain('<strong>le plan</strong>')
    expect(out).toContain('<em>quelques notes</em>')
    expect(out).toContain('<a rel="noopener noreferrer" href="https://exemple.fr/doc">le document</a>')
    expect(out).toMatch(/<ul><li><p>lundi : Réunion<\/p><\/li><li><p>mardi : .*<ul><li><p>relire<\/p><\/li><\/ul><\/li><\/ul>/)
    expect(out).toContain('<ol><li><p>premier</p></li><li><p>second</p></li></ol>')
    expect(out).toContain('<blockquote><p>Une citation</p></blockquote>')
    // The internal link of the vault is no link outside it.
    expect(out).not.toContain('href="Réunion"')
    expect(out).not.toMatch(/<(table|pre|code|img)\b/)
    expect(out).toContain('fn main() {}')
    expect(out).not.toMatch(/class=|data-|target=/)
  })

  it('from Safari drops the page styles, scripts and pixels', () => {
    const out = pasted(fixture('safari.html'))
    expect(out).toContain('<h2>Release notes</h2>')
    expect(out).toContain('<strong>new</strong>')
    expect(out).toContain('<span style="color: #2f6fd1; font-family: Georgia">faster</span>')
    // Neither the page's colours nor its sizes are ours: no span at all.
    expect(out).toContain(' and loud.')
    expect(out).not.toMatch(/rgb\(|29, 29|40px|28px|17px|position|background/)
    expect(out).not.toMatch(/<(script|img|div)\b|javascript:/)
  })

  it('from Pages keeps lines, the empty one, and a listed font', () => {
    const out = pasted(fixture('pages.html'))
    expect(out).toContain('<p>Ligne une</p><p></p>')
    expect(out).toContain('<span style="font-family: Trebuchet MS; font-size: 18px">Ligne deux</span>')
  })

  it('from Gmail and Notes keeps the lines and the simple formatting', () => {
    const gmail = pasted(fixture('gmail.html'))
    expect(gmail).toContain('<p>Salut,</p>')
    expect(gmail).toContain('<strong>le lien</strong>')
    expect(gmail).toContain('href="https://www.exemple.fr"')
    expect(gmail).toContain('Alice')
    const notes = pasted(fixture('notes.html'))
    expect(notes).toContain('<strong>Courses</strong>')
    expect(notes).toContain('<ul><li><p>pain</p></li><li><p>lait</p></li></ul>')
    expect(notes).toContain('<em>à faire avant samedi</em>')
    expect(notes).toContain('<u>important</u>')
  })

  it('of plain text keeps every line, the empty ones included', () => {
    for (const mode of ['text', 'html'] as const) {
      const { editor } = open(mode, '')
      editor.current!.view.pasteText('one\n\n\ntwo')
      expect(editor.content()).toBe(mode === 'text' ? 'one\n\n\ntwo' : '<p>one</p><p></p><p></p><p>two</p>')
    }
  })

  it('copies a line per paragraph, the empty ones included', () => {
    // A corrector (Kerlig) copies the selection and pastes its fix back:
    // the copy must be the text as written, or each line break doubles.
    for (const mode of ['text', 'html'] as const) {
      const { editor } = open(mode, '')
      editor.current!.view.pasteText('one\n\ntwo\nthree')
      const view = editor.current!.view
      const slice = view.state.doc.slice(0, view.state.doc.content.size)
      expect(view.someProp('clipboardTextSerializer', f => f(slice, view))).toBe('one\n\ntwo\nthree')
    }
  })

  it('in text only takes the text of a pasteboard with HTML too', () => {
    // A copy from Obsidian: one <p> per paragraph, and the text with the
    // empty line between them. The text wins, as a text editor pastes it.
    const { editor } = open('text', '')
    // jsdom has no DataTransfer: what ProseMirror reads of it is enough.
    const types: Record<string, string> = {
      'text/html': '<p>Un réveil.</p><p>Et dans l’après-midi.</p>',
      'text/plain': 'Un réveil.\n\nEt dans l’après-midi.',
    }
    const data = { getData: (type: string) => types[type] ?? '', types: Object.keys(types), files: [] }
    const event = new ClipboardEvent('paste')
    Object.defineProperty(event, 'clipboardData', { value: data })
    editor.current!.view.dom.dispatchEvent(event)
    expect(editor.content()).toBe('Un réveil.\n\nEt dans l’après-midi.')
  })

  it('in text only keeps the text of an HTML alone', () => {
    const { editor } = open('text', '')
    editor.current!.view.pasteHTML('<h1>Title</h1><p><strong>bold</strong> text</p><ul><li>item</li></ul>')
    expect(editor.content()).toBe('Title\nbold text\nitem')
  })
})
