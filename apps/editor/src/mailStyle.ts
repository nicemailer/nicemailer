// Colour, font and size of a run of text, each limited to the lists of
// the core (`specifications/edition.md`, section 5).
//
// Tiptap ships `Color`, `FontFamily` and `FontSize`, which accept any CSS
// value. This one extension replaces the three: each attribute lives on
// Tiptap's `textStyle` mark (a `<span style="…">`), and its `parseHTML`
// keeps only a value of the lists, normalized (`formats.ts`). A paste from
// a web page in Comic Sans, or a colour picked elsewhere, loses that style
// and keeps its text: what the editor shows is then what will leave.
//
// TypeScript note: `declare module '@tiptap/core'` adds our commands to
// Tiptap's `Commands` interface ("declaration merging"), so that
// `editor.commands.setTextColor(…)` is typed like the built-in ones.

import { Extension } from '@tiptap/core'
import { TextStyle } from '@tiptap/extension-text-style'
import type { Formats } from './formats'
import { fontStack, paletteColor, sizeValue } from './formats'

declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    mailStyle: {
      /** Sets the colour of the selection (a palette `#rrggbb`), or clears it with `null`. */
      setTextColor: (hex: string | null) => ReturnType
      /** Sets the font of the selection (a listed stack), or clears it with `null`. */
      setFont: (stack: string | null) => ReturnType
      /** Sets the size of the selection (a listed CSS value), or clears it with `null`. */
      setSize: (css: string | null) => ReturnType
    }
  }
}

export interface MailStyleOptions {
  /** Returns the lists in force (they arrive with `load`, after creation). */
  formats: () => Formats
}

/** The `style` of an element, one property read without the browser's
 * canonicalization (`element.style.color` would give `rgb(…)`; both are
 * understood, but the raw value is what the author wrote). */
function styleProperty(element: HTMLElement, name: string): string | null {
  const raw = element.getAttribute('style') ?? ''
  for (const decl of raw.split(';')) {
    const at = decl.indexOf(':')
    if (at > 0 && decl.slice(0, at).trim().toLowerCase() === name) return decl.slice(at + 1).trim()
  }
  return null
}

export const MailStyle = Extension.create<MailStyleOptions>({
  name: 'mailStyle',

  addOptions() {
    return { formats: () => ({ fonts: [], sizes: [], colors: [] }) }
  },

  addGlobalAttributes() {
    const formats = () => this.options.formats()
    return [
      {
        types: ['textStyle'],
        attributes: {
          color: {
            default: null,
            parseHTML: element => paletteColor(formats(), styleProperty(element, 'color')),
            renderHTML: attributes => (attributes.color ? { style: `color: ${attributes.color}` } : {}),
          },
          fontFamily: {
            default: null,
            parseHTML: element => fontStack(formats(), styleProperty(element, 'font-family')),
            renderHTML: attributes => (attributes.fontFamily ? { style: `font-family: ${attributes.fontFamily}` } : {}),
          },
          fontSize: {
            default: null,
            parseHTML: element => sizeValue(formats(), styleProperty(element, 'font-size')),
            renderHTML: attributes => (attributes.fontSize ? { style: `font-size: ${attributes.fontSize}` } : {}),
          },
        },
      },
    ]
  },

  addCommands() {
    // Each command sets one attribute of the `textStyle` mark, then drops
    // the mark if no attribute is left (`removeEmptyTextStyle`): a cleared
    // colour leaves no empty `<span>` behind.
    const set =
      (attribute: 'color' | 'fontFamily' | 'fontSize', allowed: (v: string) => string | null) =>
      (value: string | null) =>
      ({ chain }: { chain: () => any }) => {
        const checked = value === null ? null : allowed(value)
        if (value !== null && checked === null) return false
        return chain().setMark('textStyle', { [attribute]: checked }).removeEmptyTextStyle().run()
      }
    return {
      setTextColor: set('color', v => paletteColor(this.options.formats(), v)),
      setFont: set('fontFamily', v => fontStack(this.options.formats(), v)),
      setSize: set('fontSize', v => sizeValue(this.options.formats(), v)),
    }
  },
})

/** Whether a `<span>` carries at least one listed style. */
export function hasListedStyle(element: HTMLElement, formats: Formats): boolean {
  return (
    paletteColor(formats, styleProperty(element, 'color')) !== null ||
    fontStack(formats, styleProperty(element, 'font-family')) !== null ||
    sizeValue(formats, styleProperty(element, 'font-size')) !== null
  )
}

/**
 * Tiptap's `textStyle` mark, read only from a `<span>` that carries a
 * listed style. Tiptap's own takes any `<span style>`: a span whose styles
 * all leave would stay as an empty `<span>`, which the next load drops,
 * and a draft reopened would no longer be the same.
 */
export function mailTextStyle(formats: () => Formats) {
  return TextStyle.extend({
    parseHTML() {
      return [
        {
          tag: 'span',
          consuming: false,
          getAttrs: element => (hasListedStyle(element as HTMLElement, formats()) ? {} : false),
        },
      ]
    },
  })
}
