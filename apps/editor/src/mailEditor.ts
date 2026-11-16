// The editor of a mail's text, driven by its host through the protocol
// (`specifications/edition.md`, sections 3, 7 and 8; `PROTOCOL.md`).
//
// One `MailEditor` lives for the whole life of the page: the app keeps one
// web view, loaded at launch and hidden, and sends `load` each time a
// draft opens. The Tiptap editor inside is created again at each `load`
// and each change of mode, since the two modes have different schemas.
//
// Saving: each change raises the local revision; `changed` is posted at
// the first pause of 0.5 s (the app saves the draft then, as it did with
// its own text field). `flush` answers at once with the content as it is,
// so that `⌘↩` sends the very last key typed.

import { Editor, type JSONContent } from '@tiptap/core'
import { Fragment, Slice } from '@tiptap/pm/model'
import { TextSelection } from '@tiptap/pm/state'
import type { Formats } from './formats'
import { emptyFormats } from './formats'
import { LinkDialog } from './linkDialog'
import type { EditorState, HostMessage, LoadMessage, Mode, Strings, Theme } from './protocol'
import { PROTOCOL_VERSION } from './protocol'
import { extensions } from './schema'
import { plainDocToText, posToTextOffset, textOffsetToPos, textToParagraphs, textToPlainDoc, textToRichDoc } from './text'
import { Toolbar } from './toolbar'
import type { Transport } from './transport'

/** Pause after the last change before `changed` is posted, in ms. */
export const SAVE_PAUSE_MS = 500
/** Longest selection sent in `state` (for the AI palette). */
const MAX_SELECTION = 20_000

export class MailEditor {
  private editor: Editor | null = null
  private mode: Mode = 'text'
  private formats: Formats = emptyFormats
  private strings: Strings = {}
  private revision = 0
  /** The revision last posted (`changed` or `flushed`). */
  private posted = 0
  private timer: ReturnType<typeof setTimeout> | null = null
  private stateQueued = false
  private readonly toolbar: Toolbar
  private readonly link: LinkDialog
  private readonly mount: HTMLElement

  constructor(
    private readonly root: HTMLElement,
    private readonly transport: Transport,
  ) {
    const doc = root.ownerDocument
    this.toolbar = new Toolbar(doc, {
      editor: () => this.editor,
      openLink: () => this.openLink(),
      fontUsed: name => this.transport.send({ type: 'fontUsed', name }),
      focusChanged: () => this.queueState(),
    })
    this.link = new LinkDialog(doc, () => this.queueState())
    this.mount = doc.createElement('div')
    this.mount.className = 'nm-text'
    root.replaceChildren(this.toolbar.element, this.link.element, this.mount)
    this.toolbar.element.hidden = true
    transport.listen(message => this.receive(message))
    transport.send({ type: 'ready', v: PROTOCOL_VERSION })
  }

  /** The Tiptap editor in force, for the tests. */
  get current(): Editor | null {
    return this.editor
  }

  receive(message: HostMessage): void {
    switch (message.type) {
      case 'load':
        this.load(message)
        break
      case 'theme':
        this.applyTheme(message.theme)
        break
      case 'setMode':
        this.setMode(message.mode, message.content)
        break
      case 'replaceSelection':
        this.replaceSelection(message.text, message.html)
        break
      case 'replaceRange':
        this.replaceRange(message.start, message.end, message.text)
        break
      case 'blur':
        this.link.close()
        this.editor?.commands.blur()
        break
      case 'flush':
        this.cancelTimer()
        this.posted = this.revision
        this.transport.send({ type: 'flushed', id: message.id, content: this.content(), mode: this.mode, revision: this.revision })
        break
      case 'focus':
        this.focusNow()
        break
    }
  }

  /** The text (text only) or the HTML (formatted) of the document. */
  content(): string {
    const e = this.editor
    if (!e) return ''
    return this.mode === 'html' ? e.getHTML() : plainDocToText(e.getJSON())
  }

  private load(m: LoadMessage): void {
    if (m.v !== PROTOCOL_VERSION) return
    this.formats = m.formats
    this.strings = m.strings
    this.toolbar.build(m.formats, m.strings, m.recentFonts ?? [])
    this.link.setStrings(m.strings)
    this.applyTheme(m.theme)
    this.revision = 0
    this.posted = 0
    this.cancelTimer()
    const content = m.mode === 'text' ? textToPlainDoc(m.content) : m.fromText ? textToRichDoc(m.content) : m.content
    this.create(m.mode, content)
    const e = this.editor
    if (e && m.mode === 'text' && m.cursor) e.commands.setTextSelection(textOffsetToPos(e.state.doc, m.cursor))
    else e?.commands.setTextSelection(1)
    if (m.focus) this.focusNow()
  }

  /**
   * Gives the text the focus at once. Tiptap's `focus` command waits for
   * the next frame (`requestAnimationFrame`), and WebKit draws no frame for
   * a page it does not show: a reply opened while the window is hidden (the
   * UI scenarios, a window in another space) would never get the keys.
   */
  private focusNow(): void {
    const view = this.editor?.view
    if (view && !view.hasFocus()) view.focus()
  }

  /** Creates the Tiptap editor of `mode` with `content` (HTML or a document). */
  private create(mode: Mode, content: string | JSONContent): void {
    this.link.close()
    this.editor?.destroy()
    this.mode = mode
    this.toolbar.element.hidden = mode !== 'html'
    this.root.dataset.mode = mode
    const editor = new Editor({
      element: this.mount,
      extensions: extensions(mode, () => this.formats, { openLink: () => this.openLink(), leave: () => this.leave() }),
      content,
      editorProps: {
        attributes: {
          spellcheck: 'true',
          autocorrect: 'on',
          autocapitalize: 'sentences',
          role: 'textbox',
          'aria-multiline': 'true',
          'aria-label': this.strings['text.label'] ?? 'text.label',
        },
        // A plain text pasted keeps every line, the empty ones included:
        // one paragraph per line (ProseMirror's own parser joins runs of
        // line breaks).
        clipboardTextParser: (text, _context, _plain, view) => {
          const nodes = textToParagraphs(text).map(p => view.state.schema.nodeFromJSON(p))
          return new Slice(Fragment.from(nodes), 1, 1)
        },
        // A copy gives a line per paragraph, as the text only draft is
        // written (`plainDocToText`). Tiptap's own copy puts an empty line
        // between blocks (`\n\n`), as for paragraphs with a margin: a
        // corrector that copies the selection and pastes its fix back
        // (Kerlig) would then double every line break. Each textblock (a
        // paragraph, a heading, the text of a list item) is a line, the
        // empty ones included; a line break inside one (⇧↩) stays one.
        clipboardTextSerializer: slice => {
          const out: string[] = []
          slice.content.descendants(node => {
            if (!node.isTextblock) return true
            out.push(node.textBetween(0, node.content.size, undefined, '\n'))
            return false
          })
          return out.join('\n')
        },
        // In text only, a paste takes the pasteboard's text, never its HTML.
        // A copy from Obsidian, Safari or Notes carries both; read through
        // the text schema, the HTML would give one line per paragraph and
        // lose the empty lines between them (`<p>a</p><p>b</p>` has none),
        // when the text, the same as what a text editor pastes, has them.
        // Only a pasteboard with HTML and no text goes through the schema.
        handlePaste: (view, event) => {
          if (this.mode !== 'text') return false
          const text = event.clipboardData?.getData('text/plain')
          if (!text) return false
          const slice = view.props.clipboardTextParser!.call(view, text, view.state.selection.$from, true, view)
          view.dispatch(view.state.tr.replaceSelection(slice).scrollIntoView().setMeta('uiEvent', 'paste'))
          return true
        },
      },
      onUpdate: () => this.changedLocally(),
      onSelectionUpdate: () => this.queueState(),
      onFocus: () => this.queueState(),
      onBlur: () => this.queueState(),
    })
    this.editor = editor
    this.queueState()
  }

  private changedLocally(): void {
    this.revision += 1
    this.queueState()
    this.cancelTimer()
    this.timer = setTimeout(() => {
      this.timer = null
      this.postChanged()
    }, SAVE_PAUSE_MS)
  }

  private postChanged(): void {
    if (this.posted === this.revision) return
    this.posted = this.revision
    this.transport.send({ type: 'changed', content: this.content(), mode: this.mode, revision: this.revision })
  }

  private cancelTimer(): void {
    if (this.timer !== null) clearTimeout(this.timer)
    this.timer = null
  }

  /**
   * Changes mode. From text to formatted, the text is converted here
   * (`textToRichDoc`: lines become paragraphs, `>` lines quotations). From
   * formatted to text, the host gives the text the core made of the HTML.
   * The draft is saved at once in its new mode.
   */
  private setMode(mode: Mode, text: string): void {
    this.cancelTimer()
    this.create(mode, mode === 'html' ? textToRichDoc(text) : textToPlainDoc(text))
    this.editor?.commands.setTextSelection(1)
    this.focusNow()
    this.revision += 1
    this.postChanged()
  }

  private replaceSelection(text?: string, html?: string): void {
    const e = this.editor
    if (!e) return
    if (html !== undefined && this.mode === 'html') {
      e.chain().focus().insertContent(html).run()
    } else if (text !== undefined) {
      // The lines as an open slice (`openStart` and `openEnd` 1): the first
      // joins the paragraph before the cursor, the last the one after, as
      // a paste of the same text would. One transaction: one step of undo.
      const schema = e.state.schema
      const nodes = textToParagraphs(text).map(p => schema.nodeFromJSON(p))
      const view = e.view
      view.dispatch(view.state.tr.replaceSelection(new Slice(Fragment.from(nodes), 1, 1)).scrollIntoView())
      view.focus()
    }
  }

  /** Text only: replaces a range of the text, in one step of undo, the
   * cursor after the new words (where an AI change leaves the writing). */
  private replaceRange(start: number, end: number, text: string): void {
    const e = this.editor
    if (!e || this.mode !== 'text') return
    const view = e.view
    const doc = view.state.doc
    const from = textOffsetToPos(doc, start)
    const to = textOffsetToPos(doc, Math.max(start, end))
    const nodes = textToParagraphs(text).map(p => view.state.schema.nodeFromJSON(p))
    const tr = view.state.tr.replaceRange(from, to, new Slice(Fragment.from(nodes), 1, 1))
    // The cursor after the new words, where the writing goes on.
    tr.setSelection(TextSelection.near(tr.doc.resolve(tr.mapping.map(to, 1))))
    view.dispatch(tr.scrollIntoView())
  }

  private openLink(): void {
    if (this.editor && this.mode === 'html') this.link.open(this.editor)
  }

  private leave(): void {
    this.transport.send({ type: 'leave', direction: 'previous' })
  }

  private applyTheme(theme: Theme): void {
    const s = this.root.ownerDocument.documentElement.style
    s.setProperty('--nm-font', theme.font)
    s.setProperty('--nm-size', `${theme.size}px`)
    s.setProperty('--nm-text', theme.text)
    s.setProperty('--nm-background', theme.background)
    s.setProperty('--nm-muted', theme.muted)
    s.setProperty('--nm-accent', theme.accent)
    s.setProperty('--nm-selection', theme.selection)
    s.setProperty('--nm-border', theme.border)
    s.setProperty('--nm-panel', theme.panel)
    this.root.ownerDocument.documentElement.dataset.theme = theme.dark ? 'dark' : 'light'
  }

  /** Posts `state` once per turn of the event loop, however many changes. */
  private queueState(): void {
    if (this.stateQueued) return
    this.stateQueued = true
    queueMicrotask(() => {
      this.stateQueued = false
      const state = this.state()
      this.toolbar.update(state)
      this.transport.send(state)
    })
  }

  /** What is under the cursor. */
  state(): EditorState {
    const e = this.editor
    const style = e?.getAttributes('textStyle') ?? {}
    const block = e?.isActive('heading', { level: 1 })
      ? 'h1'
      : e?.isActive('heading', { level: 2 })
        ? 'h2'
        : e?.isActive('heading', { level: 3 })
          ? 'h3'
          : 'paragraph'
    const selection = e ? e.state.doc.textBetween(e.state.selection.from, e.state.selection.to, '\n') : ''
    const range =
      e && this.mode === 'text'
        ? { start: posToTextOffset(e.state.doc, e.state.selection.from), end: posToTextOffset(e.state.doc, e.state.selection.to) }
        : null
    return {
      type: 'state',
      modal: this.link.isOpen || this.toolbar.hasFocus(),
      focused: e?.isFocused ?? false,
      textRange: range,
      empty: e?.state.selection.empty ?? true,
      selection: selection.slice(0, MAX_SELECTION),
      marks: {
        bold: e?.isActive('bold') ?? false,
        italic: e?.isActive('italic') ?? false,
        underline: e?.isActive('underline') ?? false,
        link: e?.isActive('link') ?? false,
      },
      block,
      bulletList: e?.isActive('bulletList') ?? false,
      orderedList: e?.isActive('orderedList') ?? false,
      blockquote: e?.isActive('blockquote') ?? false,
      color: (style.color as string | undefined) ?? null,
      font: (style.fontFamily as string | undefined) ?? null,
      size: (style.fontSize as string | undefined) ?? null,
    }
  }
}
