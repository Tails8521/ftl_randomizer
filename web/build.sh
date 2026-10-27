#!/bin/sh
# Build the web page: the library for webassembly, then the glue around it.
#
#   web/build.sh
#
# Needs the target once (`rustup target add wasm32-unknown-unknown`) and the
# `wasm-bindgen` command of the same version as the library in `Cargo.toml`:
#
#   cargo install --force wasm-bindgen-cli --version 0.2.100
#
# The module is built with 128 bit SIMD; that is set in `.cargo/config.toml` for the
# webassembly target, so it needs nothing here, and it asks for a browser from 2021
# or later (Chrome 91, Firefox 89, Safari 16.4).
#
# Then serve the project and open it - the page cannot be opened from a file
# path, because a web worker and a .wasm both have to be fetched. The page is
# `index.html` at the root of the repository, so the project itself is what gets
# served, and so is what GitHub Pages serves:
#
#   python3 -m http.server 8000 -d .
set -e
cd "$(dirname "$0")/.."

want=$(sed -n 's/^wasm-bindgen = { version = "=\{0,1\}\([0-9][0-9.]*\)".*/\1/p' Cargo.toml)
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')
if [ "$have" != "$want" ]; then
	echo "wasm-bindgen $want is needed and $have${have:-none} is installed:" >&2
	echo "  cargo install --force wasm-bindgen-cli --version $want" >&2
	exit 1
fi

cargo build --release --target wasm32-unknown-unknown --lib --features web
wasm-bindgen --target web --out-dir web/pkg target/wasm32-unknown-unknown/release/ftl_randomizer.wasm

echo
echo "built web/pkg - serve the project folder and open it:"
echo "  python3 -m http.server 8000 -d ."