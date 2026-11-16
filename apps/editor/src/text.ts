// Text only, and the conversion from text to a formatted document
// (`specifications/edition.md`, section 3).
//
// In text only, the document is a list of paragraphs, one per line: the
// text is the paragraphs' text joined by line breaks, and back. An empty
// line is an empty paragraph, so no blank line is ever lost (ProseMirror's
// own plain-text paste joins runs of line breaks, which would).
//
// Going to the formatted mode, the lines that start with `>` become a
// quotation (`>>` a quotation inside one), their prefix removed: the
// quoted mail of a reply becomes a quotation block. The other way, from
// HTML to text, is the core's (`html_to_text`), the same conversion as the
// text part of the mail sent: the editor has no second one.

import type { JSONContent } from '@tiptap/core'
import type { Node as PMNode } from '@tiptap/pm/model'

/** A paragraph holding one line of text (none for an empty line). */
function paragraph(line: string): JSONContent {
  return line === '' ? { type: 'paragraph' } : { type: 'paragraph', content: [{ type: 'text', text: line }] }
}

/** Lines of a text, `\r\n` and `\r` read as `\n`. */
export function lines(text: string): string[] {
  return text.replace(/\r\n?/g, '\n').split('\n')
}

/** The document of a text in text only: one paragraph per line. */
export function textToPlainDoc(text: string): JSONContent {
  return { type: 'doc', content: lines(text).map(paragraph) }
}

/** The text of a document in text only: its paragraphs, one per line. */
export function plainDocToText(doc: JSONContent): string {
  return (doc.content ?? []).map(blockText).join('\n')
}

/** The text of a block: its text nodes, a hard break as a line break. */
function blockText(node: JSONContent): string {
  if (node.type === 'text') return node.text ?? ''
  if (node.type === 'hardBreak') return '\n'
  return (node.content ?? []).map(blockText).join('')
}

/** The quotation depth of a line (`>`, `>>`, `> >`), and its text without the prefix. */
export function quoteDepth(line: string): [number, string] {
  const m = /^((?:>\s?)+)/.exec(line)
  if (!m) return [0, line]
  const prefix = m[1] ?? ''
  const depth = (prefix.match(/>/g) ?? []).length
  return [depth, line.slice(prefix.length)]
}

/**
 * The formatted document of a text: one paragraph per line, the lines
 * starting with `>` in quotations, nested by their number of `>`.
 */
export function textToRichDoc(text: string): JSONContent {
  const root: JSONContent = { type: 'doc', content: [] }
  // The open quotations: `stack[0]` is the document, `stack[d]` the
  // quotation of depth `d`.
  const stack: JSONContent[] = [root]
  for (const line of lines(text)) {
    const [depth, rest] = quoteDepth(line)
    while (stack.length - 1 > depth) stack.pop()
    while (stack.length - 1 < depth) {
      const quote: JSONContent = { type: 'blockquote', content: [] }
      stack[stack.length - 1]?.content?.push(quote)
      stack.push(quote)
    }
    stack[stack.length - 1]?.content?.push(paragraph(rest))
  }
  return root
}

/** Paragraphs for a text inserted at the cursor (a plain paste, the AI's text). */
export function textToParagraphs(text: string): JSONContent[] {
  return lines(text).map(paragraph)
}

/**
 * The text blocks of a document (paragraphs and headings, at any depth:
 * in a list item, a quotation), each with the position where its content
 * starts. They are the lines of the document's text, in both modes: in
 * text only every paragraph is a line; formatted, the same text the core
 * measures a selection in (`nm_store::mail_html::AiView::view`).
 */
function textblocks(doc: PMNode): { node: PMNode; start: number }[] {
  const out: { node: PMNode; start: number }[] = []
  doc.descendants((node, pos) => {
    if (!node.isTextblock) return true
    out.push({ node, start: pos + 1 })
    return false
  })
  return out
}

/**
 * The document's text: a line per text block, a line break inside one
 * (`⇧↩`) a line break too. Inside a block, a character is one position
 * and one unit of the text (JavaScript strings count UTF-16 units, as
 * ProseMirror does), and a line break one of each: an offset in a block
 * is its position minus the block's start.
 */
export function documentText(doc: PMNode): string {
  return textblocks(doc)
    .map(b => b.node.textBetween(0, b.node.content.size, undefined, '\n'))
    .join('\n')
}

/**
 * The document position of an offset of the text (UTF-16 units of
 * `documentText`): the line break between two blocks counts one unit.
 */
export function textOffsetToPos(doc: PMNode, offset: number): number {
  let left = Math.max(0, offset)
  const blocks = textblocks(doc)
  for (let i = 0; i < blocks.length; i++) {
    const { node, start } = blocks[i]!
    const length = node.content.size
    if (left <= length || i === blocks.length - 1) return start + Math.min(left, length)
    left -= length + 1
  }
  return 1
}

/** The offset of the text (UTF-16 units of `documentText`) of a document
 * position; a position between blocks counts as the start of the next. */
export function posToTextOffset(doc: PMNode, pos: number): number {
  let offset = 0
  const blocks = textblocks(doc)
  for (const { node, start } of blocks) {
    const length = node.content.size
    if (pos <= start + length) return offset + Math.min(Math.max(0, pos - start), length)
    offset += length + 1
  }
  return Math.max(0, offset - 1)
}
