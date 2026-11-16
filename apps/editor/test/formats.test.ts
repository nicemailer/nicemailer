import { describe, expect, it } from 'vitest'
import { fontStack, paletteColor, sizeValue, toHex } from '../src/formats'
import { formats } from './helpers'

// The same examples as `nm_core::formats` tests: the two sides recognize
// the same spellings.
describe('formats', () => {
  it('knows a font by its first family', () => {
    expect(fontStack(formats, 'Georgia')).toBe("Georgia, 'Times New Roman', serif")
    expect(fontStack(formats, '"Trebuchet MS", Helvetica, sans-serif')).toBe("'Trebuchet MS', Helvetica, sans-serif")
    expect(fontStack(formats, 'arial')).toBe('Arial, Helvetica, sans-serif')
    expect(fontStack(formats, '"Avenir Next", Arial, sans-serif')).toBe("'Avenir Next', sans-serif")
    // A font that is not on this Mac: the editor could not show it.
    expect(fontStack(formats, 'Comic Sans MS, cursive')).toBeNull()
    expect(fontStack(formats, '')).toBeNull()
  })

  it('keeps only a listed size', () => {
    expect(sizeValue(formats, ' 18PX ')).toBe('18px')
    expect(sizeValue(formats, '17px')).toBeNull()
    expect(sizeValue(formats, 'large')).toBeNull()
  })

  it('matches a palette colour in any spelling', () => {
    expect(toHex('#ABC')).toBe('#aabbcc')
    expect(paletteColor(formats, '#2F6FD1')).toBe('#2f6fd1')
    expect(paletteColor(formats, 'rgb(47, 111, 209)')).toBe('#2f6fd1')
    expect(paletteColor(formats, 'rgb(47,111,209)')).toBe('#2f6fd1')
    expect(paletteColor(formats, '#000')).toBeNull()
    expect(paletteColor(formats, 'red')).toBeNull()
    expect(paletteColor(formats, 'rgba(47, 111, 209, 0.5)')).toBeNull()
    expect(paletteColor(formats, 'rgb(300, 0, 0)')).toBeNull()
  })
})
