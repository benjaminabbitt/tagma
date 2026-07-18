// Thin ESM ergonomic layer over the wasm-bindgen Node build
// (ARCHITECTURE.md JS/TS packaging: "wrap the generated glue in a small
// hand-written TS ergonomic layer"). No behavior beyond re-exporting the
// wasm-bindgen bindings — parseTag already returns a plain object from the
// Rust side.
export { Index, compile, parseTag } from "../wasm/node/tagma_wasm.js";
