import { describe, expect, it } from 'vitest'
import { lines, plainDocToText, quoteDepth, textToPlainDoc, textToRichDoc } from '../src/text'

describe('text only', () => {
  it('keeps every line, the empty ones included', () => {
    for (const text of ['', 'one', 'one\ntwo', 'a\n\n\nb', '\n\nOn Monday, Alice wrote:\n> Hi\n', '\ttab\t']) {
      expect(plainDocToText(textToPlainDoc(text))).toBe(text)
    }
  })

  it('reads every kind of line break', () => {
    expect(lines('a\r\nb\rc\nd')).toEqual(['a', 'b', 'c', 'd'])
  })
})

describe('text to formatted', () => {
  it('makes quotations of the quoted lines, nested', () => {
    expect(quoteDepth('> x')).toEqual([1, 'x'])
    expect(quoteDepth('>> x')).toEqual([2, 'x'])
    expect(quoteDepth('> > x')).toEqual([2, 'x'])
    expect(quoteDepth('>')).toEqual([1, ''])
    expect(quoteDepth('a > b')).toEqual([0, 'a > b'])
    const doc = textToRichDoc('Thanks.\n\nOn Monday, Alice wrote:\n> Lunch?\n>> Old\n> Yes')
    expect(doc).toEqual({
      type: 'doc',
      content: [
        { type: 'paragraph', content: [{ type: 'text', text: 'Thanks.' }] },
        { type: 'paragraph' },
        { type: 'paragraph', content: [{ type: 'text', text: 'On Monday, Alice wrote:' }] },
        {
          type: 'blockquote',
          content: [
            { type: 'paragraph', content: [{ type: 'text', text: 'Lunch?' }] },
            { type: 'blockquote', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Old' }] }] },
            { type: 'paragraph', content: [{ type: 'text', text: 'Yes' }] },
          ],
        },
      ],
    })
  })
})

describe('offsets of the text', () => {
  it('map to positions of the document and back', async () => {
    const { open } = await import('./helpers')
    const { posToTextOffset, textOffsetToPos } = await import('../src/text')
    const { editor } = open('text', 'ab\n\nc🎉d')
    const doc = editor.current!.state.doc
    // "ab" is in the first paragraph (positions 1..3), the empty line at 5,
    // "c🎉d" from 7 (the emoji is two units, as in Swift's NSString).
    expect([0, 1, 2, 3, 4, 5, 7, 8].map(o => textOffsetToPos(doc, o))).toEqual([1, 2, 3, 5, 7, 8, 10, 11])
    for (const o of [0, 1, 2, 3, 4, 5, 7, 8]) expect(posToTextOffset(doc, textOffsetToPos(doc, o))).toBe(o)
    expect(textOffsetToPos(doc, 100)).toBe(11)
  })
})
