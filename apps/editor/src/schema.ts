// The Tiptap extensions of each mode (`specifications/edition.md`,
// sections 3, 5 and 6).
//
// The schema is the editor's strongest rule: nothing enters the document
// that is not in it. A paste, a drop, a text written back by another app
// (Writing Tools, a grammar checker) are all read through it, and what it
// does not know (a table, an image, a style) leaves, its text kept.
//
// - Text only: paragraphs and text, nothing else. One paragraph is one
//   line (`text.ts`).
// - Formatted: paragraphs, line breaks, headings 1 to 3, bulleted and
//   numbered lists, quotations, bold, italic, underline, links (http,
//   https, mailto), and the colour, font and size of the lists
//   (`mailStyle.ts`). No table, image, code, strike or rule.
//
// Tiptap's typing shortcuts are kept in the formatted mode (`# ` makes a
// heading, `- ` a list, `**x**` bold): they format as one types, and no
// Markdown is ever stored.

import { Extension, type AnyExtension } from '@tiptap/core'
import Blockquote from '@tiptap/extension-blockquote'
import Bold from '@tiptap/extension-bold'
import Document from '@tiptap/extension-document'
import HardBreak from '@tiptap/extension-hard-break'
import Heading from '@tiptap/extension-heading'
import Italic from '@tiptap/extension-italic'
import Link from '@tiptap/extension-link'
import { BulletList, ListItem, ListKeymap, OrderedList } from '@tiptap/extension-list'
import Paragraph from '@tiptap/extension-paragraph'
import Text from '@tiptap/extension-text'
import Underline from '@tiptap/extension-underline'
import { Dropcursor, UndoRedo } from '@tiptap/extensions'
import type { Formats } from './formats'
import { MailStyle, mailTextStyle } from './mailStyle'
import type { Mode } from './protocol'

/** What the keys of the editor ask of its controller (`editor.ts`). */
export interface KeyHandlers {
  /** `⌘K`: the link dialog. */
  openLink: () => void
  /** `⇧⇥` outside a list: back to the subject. */
  leave: () => void
}

/** The only schemes a link may have; the core checks them again. */
export function allowedLink(url: string): boolean {
  return /^(https?:\/\/|mailto:)/i.test(url.trim())
}

/**
 * The keys of a mail's text that are not Tiptap's own. Their priority is
 * below the lists' (`ListItem` takes `⇥` and `⇧⇥` to indent and outdent),
 * so these run only when the cursor is not in a list item that can move.
 */
function keys(mode: Mode, handlers: KeyHandlers): AnyExtension {
  return Extension.create({
    name: 'mailKeys',
    priority: 50,
    addKeyboardShortcuts() {
      return {
        // Text only: a tab is typed, as it is in a text field. Formatted:
        // a tab means nothing in HTML outside a list, so nothing happens
        // (and the focus stays in the text).
        Tab: () => {
          if (mode === 'text') return this.editor.commands.insertContent('\t')
          return true
        },
        'Shift-Tab': () => {
          if (this.editor.isActive('listItem')) return true
          handlers.leave()
          return true
        },
        'Mod-k': () => {
          if (mode === 'text') return false
          handlers.openLink()
          return true
        },
      }
    },
  })
}

/** The extensions of `mode`. `formats` returns the lists in force. */
export function extensions(mode: Mode, formats: () => Formats, handlers: KeyHandlers): AnyExtension[] {
  const common: AnyExtension[] = [Document, Paragraph, Text, UndoRedo, keys(mode, handlers)]
  if (mode === 'text') return common
  return [
    ...common,
    HardBreak,
    Heading.configure({ levels: [1, 2, 3] }),
    BulletList,
    OrderedList,
    ListItem,
    ListKeymap,
    Blockquote,
    Bold,
    Italic,
    Underline,
    // A link holds its address only: the `target`, `class` and `rel` of a
    // pasted page (Obsidian marks its links `target="_blank"`) are not
    // kept, and every link is written with the same `rel`.
    Link.extend({
      addAttributes() {
        return { href: { default: null, parseHTML: (element: HTMLElement) => element.getAttribute('href') } }
      },
    }).configure({
      autolink: true,
      linkOnPaste: true,
      openOnClick: false,
      defaultProtocol: 'https',
      protocols: ['mailto'],
      // No `target`: the mail's reader decides how a link opens.
      HTMLAttributes: { target: null, rel: 'noopener noreferrer' },
      isAllowedUri: (url, ctx) => allowedLink(url) && ctx.defaultValidate(url),
      shouldAutoLink: url => allowedLink(url) || /^www\./i.test(url),
    }),
    mailTextStyle(formats),
    MailStyle.configure({ formats }),
    Dropcursor,
  ]
}
