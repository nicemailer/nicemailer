// The formatting bar (`specifications/edition.md`, section 6): font, size,
// colour, bold, italic, underline, headings, lists, quotation, link. It
// shows the formatting under the cursor, always, in the formatted mode
// (`⇧⌘T` is the switch: a formatted text without its bar would hide what
// it can do), and never in text only.
//
// The font menu offers every font of the Mac, those used recently first.
//
// It is plain DOM, rebuilt at each `load`, and updated in place at each
// change of the cursor: no framework.

import type { Editor } from '@tiptap/core'
import type { Formats } from './formats'
import type { EditorState, Strings } from './protocol'

export interface ToolbarActions {
  /** The editor in force (it is created again at each change of mode). */
  editor: () => Editor | null
  openLink: () => void
  /** A font was chosen in the menu (the host remembers the recent ones). */
  fontUsed: (name: string) => void
  /** Focus entered or left the bar (Escape then belongs to the editor). */
  focusChanged: () => void
}

/** A text of the interface, its key when the host gave none. */
function t(strings: Strings, key: string): string {
  return strings[key] ?? key
}

/** How many fonts the "Recently used" part of the menu shows. */
export const RECENT_FONTS = 5

export class Toolbar {
  readonly element: HTMLElement
  private readonly bar: HTMLElement
  private formats: Formats = { fonts: [], sizes: [], colors: [] }
  private strings: Strings = {}
  /** The fonts chosen last, most recent first. */
  private recent: string[] = []
  private buttons = new Map<string, HTMLButtonElement>()
  private selects = new Map<string, HTMLSelectElement>()

  constructor(
    private readonly doc: Document,
    private readonly actions: ToolbarActions,
  ) {
    this.element = doc.createElement('div')
    this.element.className = 'nm-toolbar'
    this.bar = doc.createElement('div')
    this.bar.className = 'nm-bar'
    this.bar.setAttribute('role', 'toolbar')
    this.element.append(this.bar)
    this.element.addEventListener('focusin', () => this.actions.focusChanged())
    this.element.addEventListener('focusout', () => setTimeout(() => this.actions.focusChanged(), 0))
    // Escape in the bar gives the keys back to the text.
    this.element.addEventListener('keydown', e => {
      if (e.key === 'Escape') {
        e.preventDefault()
        this.actions.editor()?.commands.focus()
      }
    })
  }

  /** Whether the focus is in the bar (one of its menus or buttons). */
  hasFocus(): boolean {
    return this.element.contains(this.doc.activeElement)
  }

  /** The names of the "Recently used" part of the font menu. */
  recentFonts(): string[] {
    return [...this.recent]
  }

  /** Builds the controls for these lists, texts and recent fonts. */
  build(formats: Formats, strings: Strings, recent: string[]): void {
    this.formats = formats
    this.strings = strings
    // Only the fonts of the list: a font removed from the Mac since leaves.
    this.recent = recent.filter(name => formats.fonts.some(f => f.name === name)).slice(0, RECENT_FONTS)
    this.bar.replaceChildren()
    this.buttons.clear()
    this.selects.clear()
    const run = (f: (e: Editor) => void) => () => {
      const e = this.actions.editor()
      if (e) f(e)
    }
    const select = (name: string, label: string, options: [string, string][], apply: (e: Editor, v: string | null) => void) => {
      const s = this.doc.createElement('select')
      s.className = `nm-select nm-${name}`
      s.title = label
      s.setAttribute('aria-label', label)
      for (const [value, text] of options) {
        const o = this.doc.createElement('option')
        o.value = value
        o.textContent = text
        s.append(o)
      }
      s.addEventListener('change', () => {
        const e = this.actions.editor()
        if (!e) return
        apply(e, s.value === '' ? null : s.value)
        e.commands.focus()
      })
      this.selects.set(name, s)
      this.bar.append(s)
    }
    const button = (name: string, label: string, text: string, onClick: () => void) => {
      const b = this.doc.createElement('button')
      b.type = 'button'
      b.className = `nm-button nm-${name}`
      b.title = label
      b.setAttribute('aria-label', label)
      b.textContent = text
      // The text keeps its selection: a click on a button must not take
      // the focus from it.
      b.addEventListener('mousedown', e => e.preventDefault())
      b.addEventListener('click', onClick)
      this.buttons.set(name, b)
      this.bar.append(b)
    }
    const separator = () => {
      const s = this.doc.createElement('span')
      s.className = 'nm-separator'
      this.bar.append(s)
    }
    select('font', t(strings, 'toolbar.font'), [], (e, v) => {
      e.chain().focus().setFont(v).run()
      const font = v === null ? undefined : formats.fonts.find(f => f.stack === v)
      if (font) this.used(font.name)
    })
    this.fillFonts()
    select(
      'size',
      t(strings, 'toolbar.size'),
      [['', t(strings, 'size.normal')], ...formats.sizes.map(s => [s.css, s.key] as [string, string])],
      (e, v) => e.chain().focus().setSize(v).run(),
    )
    select(
      'color',
      t(strings, 'toolbar.color'),
      [['', t(strings, 'color.default')], ...formats.colors.map(c => [c.hex, t(strings, `color.${c.key}`)] as [string, string])],
      (e, v) => e.chain().focus().setTextColor(v).run(),
    )
    for (const c of formats.colors) {
      const option = [...(this.selects.get('color')?.options ?? [])].find(o => o.value === c.hex)
      if (option) option.style.color = c.hex
    }
    separator()
    button('bold', t(strings, 'toolbar.bold'), 'B', run(e => e.chain().focus().toggleBold().run()))
    button('italic', t(strings, 'toolbar.italic'), 'I', run(e => e.chain().focus().toggleItalic().run()))
    button('underline', t(strings, 'toolbar.underline'), 'U', run(e => e.chain().focus().toggleUnderline().run()))
    separator()
    for (const level of [1, 2, 3] as const) {
      button(`h${level}`, t(strings, `toolbar.h${level}`), `H${level}`, run(e => e.chain().focus().toggleHeading({ level }).run()))
    }
    separator()
    button('bulletList', t(strings, 'toolbar.bulletList'), '•', run(e => e.chain().focus().toggleBulletList().run()))
    button('orderedList', t(strings, 'toolbar.orderedList'), '1.', run(e => e.chain().focus().toggleOrderedList().run()))
    button('blockquote', t(strings, 'toolbar.blockquote'), '❝', run(e => e.chain().focus().toggleBlockquote().run()))
    separator()
    button('link', t(strings, 'toolbar.link'), '🔗', () => this.actions.openLink())
  }

  /** A font chosen: first of the recent ones, the menu filled again. */
  private used(name: string): void {
    this.recent = [name, ...this.recent.filter(n => n !== name)].slice(0, RECENT_FONTS)
    this.actions.fontUsed(name)
    const s = this.selects.get('font')
    const value = s?.value ?? ''
    this.fillFonts()
    if (s) s.value = value
  }

  /**
   * The font menu: "Default", then the fonts used recently, then all the
   * fonts, each group under its title (`<optgroup>`, a section of the
   * Mac's menu). A recent font is in both groups, with the same value: the
   * menu shows the first one chosen.
   */
  private fillFonts(): void {
    const s = this.selects.get('font')
    if (!s) return
    const option = (value: string, text: string, font?: string) => {
      const o = this.doc.createElement('option')
      o.value = value
      o.textContent = text
      // Each name in its own face, as in Mail's menu.
      if (font) o.style.fontFamily = font
      return o
    }
    const group = (label: string, fonts: Formats['fonts']) => {
      const g = this.doc.createElement('optgroup')
      g.label = label
      for (const f of fonts) g.append(option(f.stack, f.name, f.stack))
      return g
    }
    s.replaceChildren(option('', t(this.strings, 'font.default')))
    const recent = this.recent.map(name => this.formats.fonts.find(f => f.name === name)).filter(f => f !== undefined)
    if (recent.length > 0) s.append(group(t(this.strings, 'font.recent'), recent))
    s.append(group(t(this.strings, 'font.all'), this.formats.fonts))
  }

  /** Shows the formatting under the cursor. */
  update(state: EditorState): void {
    const on: Record<string, boolean> = {
      bold: state.marks.bold,
      italic: state.marks.italic,
      underline: state.marks.underline,
      link: state.marks.link,
      h1: state.block === 'h1',
      h2: state.block === 'h2',
      h3: state.block === 'h3',
      bulletList: state.bulletList,
      orderedList: state.orderedList,
      blockquote: state.blockquote,
    }
    for (const [name, b] of this.buttons) b.setAttribute('aria-pressed', String(on[name] ?? false))
    const set = (name: string, value: string | null) => {
      const s = this.selects.get(name)
      if (s && !this.element.contains(this.doc.activeElement)) s.value = value ?? ''
    }
    set('font', state.font)
    set('size', state.size)
    set('color', state.color)
  }
}
