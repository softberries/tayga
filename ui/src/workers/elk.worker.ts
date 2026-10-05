/**
 * Web Worker that runs ELK off the main thread. elkjs's worker build registers its own
 * `onmessage` handler when loaded in a worker; `features/map/layout.ts` talks to it through
 * elkjs's promise API (`elk-api`), so the 1.6 MB layout engine never blocks the page.
 */
import 'elkjs/lib/elk-worker.min.js'
