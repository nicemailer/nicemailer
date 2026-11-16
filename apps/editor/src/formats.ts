// The closed lists of the formatted mode: fonts, sizes, colours
// (`specifications/edition.md`, section 5).
//
// The lists themselves come from the core (`compose_formats`, defined once
// in `nm_core::formats`) through the host's `load` message: the editor
// offers exactly what the core lets through when the mail leaves. This
// file only *recognizes* a value read from HTML (a paste, a draft written
// elsewhere), which says the same font or colour in many spellings, and
// maps it to the one canonical value of the list, or to nothing. The rules
// are those of `nm_core::formats` (`font_stack`, `size_value`,
// `palette_color`); a test of each side checks the same examples.

/** A font: the name shown, and the CSS stack written in the mail. */
export interface Font {
  name: string
  stack: string
}

/** A size: a key the host translates (`small`), and its CSS value. */
export interface Size {
  key: string
  css: string
}

/** A colour of the palette: a key the host translates, and `#rrggbb`. */
export interface Color {
  key: string
  hex: string
}

export interface Formats {
  fonts: Font[]
  sizes: Size[]
  colors: Color[]
}

/** No list at all: what the editor knows before `load`. */
export const emptyFormats: Formats = { fonts: [], sizes: [], colors: [] }

/**
 * The stack of the listed font a `font-family` value names: its first
 * family decides, quotes and case ignored.
 */
export function fontStack(formats: Formats, value: string | null | undefined): string | null {
  if (!value) return null
  const first = (value.split(',')[0] ?? '').trim().replace(/^["']|["']$/g, '').trim().toLowerCase()
  return formats.fonts.find(f => f.name.toLowerCase() === first)?.stack ?? null
}

/** The CSS value of a listed size (`18px`, any case and spacing). */
export function sizeValue(formats: Formats, value: string | null | undefined): string | null {
  if (!value) return null
  const v = value.trim().toLowerCase()
  return formats.sizes.find(s => s.css.toLowerCase() === v)?.css ?? null
}

/** `#rrggbb` of a CSS colour written `#rgb`, `#rrggbb` or `rgb(r, g, b)`. */
export function toHex(value: string): string | null {
  const v = value.trim().toLowerCase()
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/.exec(v)
  if (hex) {
    const h = hex[1] ?? ''
    return '#' + (h.length === 3 ? h.split('').map(c => c + c).join('') : h)
  }
  const rgb = /^rgb\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*\)$/.exec(v)
  if (rgb) {
    const parts = rgb.slice(1, 4).map(Number)
    if (parts.some(n => n > 255)) return null
    return '#' + parts.map(n => n.toString(16).padStart(2, '0')).join('')
  }
  return null
}

/** The palette colour a CSS colour is, if it is one. */
export function paletteColor(formats: Formats, value: string | null | undefined): string | null {
  if (!value) return null
  const hex = toHex(value)
  return formats.colors.find(c => c.hex === hex)?.hex ?? null
}
