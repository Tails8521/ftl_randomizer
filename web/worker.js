// The randomizer runs here, so the page keeps answering clicks while it works.
//
// The archive is never uploaded: the bytes come from the dropped file, the two
// files go back out as a download.

import init, { open } from './pkg/ftl_randomizer.js';

/** The wasm module, fetched as the worker starts rather than when a file is dropped:
 * it is 650 KiB, and a person should not have to wait for it after they let go of
 * the file. A drop that beats the fetch waits for it below, which is what it did
 * anyway. `web/build.sh` is what puts the module in `web/pkg`. */
const loading = init({ module_or_path: './pkg/ftl_randomizer_bg.wasm' });

// A module that cannot be fetched or read is worth saying out loud now, before
// someone chooses a 270 MiB file for nothing. The report hangs off a second promise
// so `loading` itself stays rejected and every message after it fails the ordinary
// way.
loading.catch((error) => postMessage({ type: 'wasm-error', message: text(error) }));

/** The loaded archive, which holds the game bytes until the page is done. */
let job = null;

self.onmessage = async (event) => {
  const message = event.data;
  try {
    await loading;

    if (message.type === 'open') {
      draw(message.buffer, message.seed);
    } else if (message.type === 'reseed') {
      redraw(message.seed);
    } else if (message.type === 'dat') {
      say('dat', { dat: loaded().dat() });
    } else {
      throw new Error(`unknown message ${message.type}`);
    }
  } catch (error) {
    postMessage({ type: 'error', message: text(error) });
  }
};

/** Read an `ftl.dat` and draw the ships. The archive stays loaded for the `.dat`
 * and for the next seed. */
function draw(buffer, seed) {
  const magic = new TextDecoder().decode(new Uint8Array(buffer, 0, 4));
  if (magic !== 'PKG\n') {
    throw new Error(
      'That is not an ftl.dat: it does not start with "PKG". A .ftl mod is not an ftl.dat.',
    );
  }

  job?.free();
  job = open(new Uint8Array(buffer), seed);
  ready();
}

/** Draw the ships again from a new seed, with the archive that is loaded. */
function redraw(seed) {
  loaded().reseed(seed);
  ready();
}

/** The archive that is loaded. A message that arrives without one - a page that
 * was reloaded and given back its buttons - has to say so. */
function loaded() {
  if (!job) {
    throw new Error('Nothing is loaded: choose an ftl.dat first.');
  }
  return job;
}

/** Tell the page what was just drawn. */
function ready() {
  say('ready', {
    seed: job.seed(),
    changed: job.changed(),
    warnings: job.warnings(),
    ftl: job.ftl(),
  });
}

/** Post a message, transferring the big buffers it carries instead of copying. */
function say(type, fields) {
  const transfer = Object.values(fields)
    .filter((value) => value instanceof Uint8Array)
    .map((value) => value.buffer);
  postMessage({ type, ...fields }, transfer);
}

/** An error a person can act on. */
function text(error) {
  return String(error?.message ?? error).replace(/^RuntimeError: /, '');
}