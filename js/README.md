# `opera-js` (WebAssembly bindings for `opera-rs`)

Pure-Rust WebAssembly bindings for [`opera-rs`](https://github.com/opengeos/opera-rs), powering the interactive [Browser Playground](https://opera-rs.opengeos.org/playground/) and usable in browsers or Node.js with zero GDAL or C/C++ dependencies.

## Build locally

```sh
# Browser ES module (used by the Playground)
wasm-pack build --target web --release --out-dir ../site/wasm

# Node.js package + smoke test
wasm-pack build --target nodejs --release --out-dir pkg-node
node test.mjs
```
