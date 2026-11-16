// How messages travel between the editor and its host.
//
// - In the app, the editor runs in a `WKWebView`: it posts to the message
//   handler `editor` that the app registered
//   (`window.webkit.messageHandlers.editor`), and the app calls
//   `window.nicemailerEditor.receive(message)` to talk to it.
// - In a web client, the editor runs in an `<iframe>`: it posts to its
//   parent window, and listens to the parent's `message` events.
// - In the tests, a fake transport records what is posted.
//
// Only this file knows about WebKit; the rest of the editor sees a
// `Transport`.

import type { EditorMessage, HostMessage } from './protocol'

export interface Transport {
  send(message: EditorMessage): void
  /** Calls `handler` with each message of the host. */
  listen(handler: (message: HostMessage) => void): void
}

interface WebKitWindow {
  webkit?: { messageHandlers?: { editor?: { postMessage: (m: unknown) => void } } }
  nicemailerEditor?: { receive: (m: HostMessage) => void }
}

/** The transport of the page the editor runs in. */
export function pageTransport(win: Window): Transport {
  const w = win as unknown as WebKitWindow & Window
  const webkit = w.webkit?.messageHandlers?.editor
  return {
    send(message) {
      if (webkit) webkit.postMessage(message)
      else if (w.parent && w.parent !== w) w.parent.postMessage(message, '*')
    },
    listen(handler) {
      // The app calls this function through `evaluateJavaScript`.
      w.nicemailerEditor = { receive: handler }
      if (!webkit) {
        w.addEventListener('message', event => {
          if (event.source === w.parent && event.data && typeof event.data.type === 'string') handler(event.data as HostMessage)
        })
      }
    },
  }
}
