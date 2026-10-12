// Runs Ghostty's unmodified wasm examples (examples/wasm/*.html) under node
// with a minimal DOM stub, against the Rust-built ghostty-vt wasm module.
//
//   node run.mjs exports <wasm> <names-file>
//   node run.mjs sgr <sgr.html> <wasm> <sgr.h> [fix-tags]
//   node run.mjs key <key-encode.html> <wasm>
//
// Output is line-oriented for the Rust test to parse.
import fs from 'node:fs';
import vm from 'node:vm';

const [mode, ...args] = process.argv.slice(2);

function script(html) {
  const m = html.match(/<script>([\s\S]*)<\/script>/);
  if (!m) throw new Error('no <script> in html');
  return m[1];
}

function el(extra = {}) {
  const listeners = {};
  return {
    textContent: '', className: '', value: '', checked: false, disabled: true,
    style: {}, listeners,
    addEventListener(type, fn) { (listeners[type] ??= []).push(fn); },
    focus() {},
    ...extra,
  };
}

/** Load an example page into a fresh context; returns helpers. */
async function load(htmlPath, wasmPath, transform = (s) => s) {
  const html = fs.readFileSync(htmlPath, 'utf8');
  const elements = {};
  // Initial textarea contents (the sgr example's default sequence).
  const ta = html.match(/<textarea id="(\w+)"[^>]*>([^<]*)<\/textarea>/);
  if (ta) elements[ta[1]] = el({ value: ta[2] });
  const winListeners = {};
  const document = {
    getElementById(id) { return (elements[id] ??= el()); },
    querySelector() { return elements.__action ??= el({ value: '1', parentElement: { textContent: 'Press' } }); },
    querySelectorAll(sel) {
      if (sel.includes('checkbox')) return Object.keys(elements).filter((k) => k.startsWith('flag_')).map((k) => elements[k]);
      return [];
    },
  };
  const window = {
    location: { protocol: 'http:' },
    addEventListener(type, fn) { (winListeners[type] ??= []).push(fn); },
  };
  const wasmBytes = fs.readFileSync(wasmPath);
  const fetch = async () => ({ arrayBuffer: async () => wasmBytes.buffer.slice(wasmBytes.byteOffset, wasmBytes.byteOffset + wasmBytes.byteLength) });
  const ctx = vm.createContext({
    document, window, fetch, console, TextEncoder, TextDecoder, DataView, Uint8Array, Uint16Array,
    WebAssembly, Array, Object, parseInt, isNaN, Error, String,
  });
  vm.runInContext(transform(script(html)), ctx, { filename: htmlPath });
  return { ctx, elements, winListeners, document };
}

async function runInit(page) {
  for (const fn of page.winListeners.DOMContentLoaded ?? []) await fn();
}

function sgrTagsFromHeader(headerPath) {
  const h = fs.readFileSync(headerPath, 'utf8');
  const out = [];
  for (const m of h.matchAll(/GHOSTTY_SGR_ATTR_([A-Z0-9_]+)\s*=\s*(\d+)/g)) out.push(`${m[1]}: ${m[2]}`);
  return `const SGR_ATTR_TAGS = {\n${out.join(',\n')}\n};`;
}

if (mode === 'exports') {
  const [wasmPath, namesFile] = args;
  const mod = new WebAssembly.Module(fs.readFileSync(wasmPath));
  const exported = new Set(WebAssembly.Module.exports(mod).map((e) => e.name));
  const imports = WebAssembly.Module.imports(mod).map((i) => `${i.module}.${i.name}`);
  console.log(`imports: ${imports.join(',')}`);
  console.log(`has_memory: ${exported.has('memory')}`);
  for (const n of fs.readFileSync(namesFile, 'utf8').split('\n').filter(Boolean)) {
    if (!exported.has(n)) console.log(`MISSING ${n}`);
  }
  console.log('done');
} else if (mode === 'sgr') {
  const [html, wasm, header, fix] = args;
  const transform = fix === 'fix-tags'
    ? (s) => s.replace(/const SGR_ATTR_TAGS = \{[\s\S]*?\};/, sgrTagsFromHeader(header))
    : (s) => s;
  const page = await load(html, wasm, transform);
  await runInit(page);
  const out = page.elements.output;
  const seq = page.elements.sequence;
  console.log('=== default');
  console.log(out.textContent);
  for (const s of ['1;3;31;42', '38:5:123;48;5;200', '0', '4:0;4:1;4:2;4:3;4:4;4:5', '9999', '91;102;53']) {
    seq.value = s;
    for (const fn of seq.listeners.input ?? []) fn();
    console.log(`=== ${s}`);
    console.log(out.textContent);
  }
} else if (mode === 'key') {
  const [html, wasm] = args;
  const page = await load(html, wasm);
  await runInit(page);
  const out = page.elements.output;
  const input = page.elements.keyInput;
  const press = (name, ev) => {
    const e = { key: 'a', code: 'KeyA', shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, preventDefault() {}, ...ev };
    for (const fn of input.listeners.keydown ?? []) fn(e);
    const m = out.textContent.match(/^Hex: (.*)$/m);
    console.log(`${name}\t${m ? m[1].trim() : 'NONE'}`);
  };
  const setAction = (v, label) => {
    const r = page.document.querySelector();
    r.value = String(v); r.parentElement.textContent = label;
  };
  const setFlags = (on) => {
    for (const id of Object.keys(page.elements).filter((k) => k.startsWith('flag_'))) page.elements[id].checked = on;
    for (const id of Object.keys(page.elements).filter((k) => k.startsWith('flag_'))) {
      for (const fn of page.elements[id].listeners.change ?? []) fn();
    }
  };
  // The page creates the checkbox elements lazily via getElementById, which
  // happens in init(); make sure they start checked (as in the HTML).
  setFlags(true);
  console.log('flags: all');
  press('ctrl+a', { ctrlKey: true });
  press('a', {});
  press('shift+Enter', { key: 'Enter', code: 'Enter', shiftKey: true });
  press('ArrowUp', { key: 'ArrowUp', code: 'ArrowUp' });
  press('alt+x', { key: 'x', code: 'KeyX', altKey: true });
  setAction(0, 'Release');
  press('release a', {});
  setAction(1, 'Press');
  setFlags(false);
  console.log('flags: none');
  press('ctrl+a', { ctrlKey: true });
  press('a', {});
  press('shift+Enter', { key: 'Enter', code: 'Enter', shiftKey: true });
  press('ArrowUp', { key: 'ArrowUp', code: 'ArrowUp' });
  press('alt+x', { key: 'x', code: 'KeyX', altKey: true });
  press('Escape', { key: 'Escape', code: 'Escape' });
  press('F5', { key: 'F5', code: 'F5' });
  press('unidentified', { key: 'Dead', code: 'Foo' });
} else {
  throw new Error(`unknown mode ${mode}`);
}
