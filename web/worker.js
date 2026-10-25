// The randomizer runs here, so the page keeps answering clicks while it works.
//
// The archive is never uploaded: the bytes come from the dropped file, the two
// files go back out as a download.

import init, { open } from './pkg/ftl_randomizer.js';

/** The wasm module, loaded once. */
let loading = null;
/** The loaded archive, which holds the game bytes until the page is done. */
let job = null;

self.onmessage = async (event) => {
  const message = event.data;
  try {
    loading ??= init({ module_or_path: './pkg/ftl_randomizer_bg.wasm' });
    await loading;

    if (message.type === 'open') {
      draw(message.buffer, message.seed);
    } else if (message.type === 'dat') {
      say('dat', { dat: job.dat() });
    } else {
      throw new Error(`unknown message ${message.type}`);
    }
  } catch (error) {
    postMessage({ type: 'error', message: text(error) });
  }
};

/** Read an `ftl.dat` and draw the ships. The archive stays loaded for the `.dat`. */
function draw(buffer, seed) {
  const magic = new TextDecoder().decode(new Uint8Array(buffer, 0, 4));
  if (magic !== 'PKG\n') {
    throw new Error(
      'That is not an ftl.dat: it does not start with "PKG". A .ftl mod is not an ftl.dat.',
    );
  }

  job?.free();
  job = open(new Uint8Array(buffer), seed);
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