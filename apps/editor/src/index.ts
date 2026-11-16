// Entry point of the bundle (`build.mjs`): the editor of the page, on the
// element `#editor`, talking to the page's host.

import { MailEditor } from './mailEditor'
import { pageTransport } from './transport'

const root = document.getElementById('editor')
if (root) new MailEditor(root, pageTransport(window))
