// The link dialog (`⌘K`, `specifications/edition.md`, section 6): the
// address, and the text when nothing is selected. A scheme other than
// http, https or mailto is refused; an address written without one gets
// `https://` (`mailto:` when it is an email address).

import type { Editor } from '@tiptap/core'
import type { Strings } from './protocol'
import { allowedLink } from './schema'

/** The address as the link will hold it, or `null` if it cannot be one. */
export function normalizeLink(input: string): string | null {
  const v = input.trim()
  if (v === '') return null
  if (allowedLink(v)) return v
  if (/^[a-z][a-z0-9+.-]*:/i.test(v)) return null
  if (/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(v)) return `mailto:${v}`
  if (/^[^\s]+\.[^\s]+/.test(v)) return `https://${v}`
  return null
}

export class LinkDialog {
  readonly element: HTMLElement
  private readonly url: HTMLInputElement
  private readonly text: HTMLInputElement
  private readonly error: HTMLElement
  private editor: Editor | null = null

  constructor(
    private readonly doc: Document,
    private readonly changed: () => void,
  ) {
    this.element = doc.createElement('form')
    this.element.className = 'nm-dialog'
    this.element.hidden = true
    this.url = doc.createElement('input')
    this.url.type = 'text'
    this.url.className = 'nm-link-url'
    this.url.spellcheck = false
    this.text = doc.createElement('input')
    this.text.type = 'text'
    this.text.className = 'nm-link-text'
    this.error = doc.createElement('div')
    this.error.className = 'nm-error'
    this.element.append(this.text, this.url, this.error)
    this.element.addEventListener('submit', e => {
      e.preventDefault()
      this.apply()
    })
    this.element.addEventListener('keydown', e => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        this.close()
      } else if (e.key === 'Enter') {
        e.preventDefault()
        this.apply()
      }
    })
  }

  get isOpen(): boolean {
    return !this.element.hidden
  }

  setStrings(strings: Strings): void {
    this.url.placeholder = strings['link.url'] ?? 'link.url'
    this.text.placeholder = strings['link.text'] ?? 'link.text'
    this.url.setAttribute('aria-label', this.url.placeholder)
    this.text.setAttribute('aria-label', this.text.placeholder)
    this.error.dataset.message = strings['link.invalid'] ?? 'link.invalid'
  }

  open(editor: Editor): void {
    this.editor = editor
    const current = editor.getAttributes('link').href as string | undefined
    // On an existing link, its whole extent is edited.
    if (current) editor.chain().extendMarkRange('link').run()
    this.url.value = current ?? ''
    this.text.hidden = !editor.state.selection.empty || !!current
    this.text.value = ''
    this.error.textContent = ''
    this.element.hidden = false
    this.changed()
    ;(this.text.hidden ? this.url : this.text).focus()
  }

  close(): void {
    if (this.element.hidden) return
    this.element.hidden = true
    this.changed()
    this.editor?.commands.focus()
  }

  private apply(): void {
    const editor = this.editor
    if (!editor) return
    // An empty address removes the link.
    if (this.url.value.trim() === '') {
      editor.chain().focus().extendMarkRange('link').unsetLink().run()
      this.close()
      return
    }
    const href = normalizeLink(this.url.value)
    if (!href) {
      this.error.textContent = this.error.dataset.message ?? ''
      return
    }
    if (!this.text.hidden) {
      const label = this.text.value.trim() || this.url.value.trim()
      editor
        .chain()
        .focus()
        .insertContent({ type: 'text', text: label, marks: [{ type: 'link', attrs: { href } }] })
        .unsetMark('link')
        .run()
    } else {
      editor.chain().focus().extendMarkRange('link').setLink({ href }).run()
    }
    this.close()
  }
}
