// Thin ESM ergonomic layer over the wasm-bindgen web build (browser/edge
// targets). Consumers must await the default export (the wasm-bindgen
// `init` function, for streaming instantiation) before calling
// Index/compile/parseTag — this wrapper changes nothing about that contract
// (ARCHITECTURE.md JS/TS packaging).
export {
  default,
  initSync,
  Index,
  compile,
  parseTag,
} from "../wasm/web/tagma_wasm.js";
