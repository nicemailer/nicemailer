// What jsdom lacks and ProseMirror's paste functions use: a `paste` event
// is only passed along to the handlers, so a bare event is enough.
if (typeof globalThis.ClipboardEvent === 'undefined') {
  class FakeClipboardEvent extends Event {
    clipboardData: DataTransfer | null = null
  }
  ;(globalThis as unknown as { ClipboardEvent: typeof FakeClipboardEvent }).ClipboardEvent = FakeClipboardEvent
}
// ProseMirror measures the layout to scroll the cursor into view; jsdom
// has no layout, so every rectangle is empty.
if (!document.createRange().getClientRects) {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
}
Element.prototype.scrollIntoView ??= () => {}
