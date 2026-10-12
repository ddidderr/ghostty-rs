// Instantiates ghostty_wasm.wasm with stub JS imports and exercises the
// exported config / atlas / canvas face / shaper functions. Run by
// tests/node.rs: `node node_test.js <path-to-wasm>`.
const fs = require('fs');
const assert = require('assert');
const path = require('path');
// zig-js's own JS host (vendored, unmodified TypeScript; node strips types).
const { ZigJS } = require(path.join(__dirname, '..', 'js', 'zig-js', 'src', 'zigjs.ts'));

// A fake canvas: every glyph is a 7px wide, 12px ascent, 4px descent box
// (emoji are 14px wide and "colored"). measureText records the font in use.
class FakeCanvas {
  constructor() {
    this.width = 300;
    this.height = 150;
    this.style = {};
    this.calls = [];
    const self = this;
    this.ctx = {
      font: '',
      textBaseline: 'alphabetic',
      fillStyle: '',
      clearRect: (...a) => {
        self.calls.push(['clearRect', ...a]);
      },
      fillRect: (...a) => {
        self.calls.push(['fillRect', ...a]);
      },
      fillText: (t, x, y) => {
        self.lastText = { t, x, y };
        self.calls.push(['fillText', t, x, y]);
      },
      measureText: (t) => {
        const n = [...t].length;
        const emoji = /\p{Extended_Pictographic}/u.test(t);
        const w = emoji ? 14 : 7 * n;
        return {
          width: w + 1,
          actualBoundingBoxLeft: 0,
          actualBoundingBoxRight: w,
          actualBoundingBoxAscent: 12,
          actualBoundingBoxDescent: t.includes('_') ? 4 : 0.5,
          _t: t,
        };
      },
      getImageData: (x, y, w, h) => {
        const data = new Uint8ClampedArray(w * h * 4);
        const emoji = /\p{Extended_Pictographic}/u.test(self.lastText ? self.lastText.t : '');
        for (let i = 0; i < w * h; i++) {
          if (emoji) {
            data[i * 4] = 200; // red channel => color
            data[i * 4 + 3] = 255;
          } else {
            data[i * 4 + 3] = (i % 2) * 255; // alpha only (black text)
          }
        }
        return { data };
      },
      putImageData: (...a) => {
        self.calls.push(['putImageData', ...a]);
      },
    };
  }
  getContext(kind) {
    assert.strictEqual(kind, '2d');
    return this.ctx;
  }
}
class FakeImageData {
  constructor(data, w, h) {
    this.data = data;
    this.width = w;
    this.height = h;
  }
}
const canvases = [];
// zig-js reads `globalThis` directly (predefined ref 5).
globalThis.document = {
  createElement: (tag) => {
    assert.strictEqual(tag, 'canvas');
    const c = new FakeCanvas();
    canvases.push(c);
    return c;
  },
  getElementById: () => null,
};
globalThis.ImageData = FakeImageData;

// Live (non-predefined) entries in the zig-js value table, to catch leaks.
const liveValues = (zigjs) => zigjs.values.slice(7).filter((v) => v !== null).length;

(async () => {
  const logs = [];
  const zigjs = new ZigJS();
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  let instance;
  const mem = () => new Uint8Array(instance.exports.memory.buffer);
  const readString = (ptr, len) => dec.decode(mem().subarray(ptr, ptr + len));
  const imports = {
    ...zigjs.importObject(),
    env: { log: (ptr, len) => logs.push(readString(ptr, len)) },
  };
  // The import module must be exactly zig-js's.
  assert.deepStrictEqual(Object.keys(imports).sort(), ['env', 'zig-js']);
  const bytes = fs.readFileSync(process.argv[2]);
  ({ instance } = await WebAssembly.instantiate(bytes, imports));
  zigjs.memory = instance.exports.memory;
  const wanted = Object.keys(zigjs.importObject()['zig-js']).sort();
  const got = WebAssembly.Module.imports(new WebAssembly.Module(bytes))
    .filter((i) => i.module === 'zig-js')
    .map((i) => i.name);
  assert.ok(got.every((n) => wanted.includes(n)), `unexpected zig-js imports: ${got}`);
  const host = {
    writeString(s) {
      const b = enc.encode(s);
      const ptr = instance.exports.wasm_alloc(b.length);
      mem().set(b, ptr);
      return { ptr, len: b.length, free: () => instance.exports.wasm_free(ptr, b.length) };
    },
    readString,
  };
  const x = instance.exports;
  x.ghostty_wasm_init();

  // ---- config ----
  {
    const cfg = x.config_new();
    assert.ok(cfg);
    const s = host.writeString('font-size = 18\nwindow-padding-x = 7\nwindow-width = 1\nnot-a-key = 1\n');
    x.config_load_string(cfg, s.ptr, s.len);
    s.free();
    assert.strictEqual(x.config_diagnostics_count(cfg), 1);
    x.config_finalize(cfg);
    assert.deepStrictEqual(logs, [], 'config finalization must succeed without desktop shell lookup');
    const cap = 1 << 16;
    const out = x.wasm_alloc(cap);
    const n = x.config_format(cfg, out, cap);
    assert.ok(n > 0 && n <= cap);
    const text = host.readString(out, n);
    x.wasm_free(out, cap);
    assert.match(text, /font-size = 18/);
    assert.match(text, /window-padding-x = 7/);
    // Width clamping happens after shell defaults: finalization must reach it.
    assert.match(text, /window-width = 10\n/);
    x.config_free(cfg);
  }

  // ---- atlas ----
  const atlas = x.atlas_new(64, 0); // grayscale
  assert.ok(atlas);
  assert.strictEqual(x.atlas_size(atlas), 64);

  // ---- face ----
  const f = host.writeString('monospace');
  const face = x.face_new(f.ptr, f.len, 12, 0);
  assert.ok(face, 'face_new ' + logs.join('|'));
  const m = x.wasm_alloc(12);
  x.face_metrics(face, m);
  const metrics = new Uint32Array(instance.exports.memory.buffer, m, 3).slice();
  assert.deepStrictEqual([...metrics], [7, 16, 4]);
  assert.ok(canvases[0].ctx.font === '12px monospace', canvases[0].ctx.font);

  // Text presentation face supports ascii/latin text but not color emoji
  assert.strictEqual((x.face_glyph_index(face, 0x41) >>> 0), 0x41);
  assert.strictEqual((x.face_glyph_index(face, 0x4e2d) >>> 0), 0x4e2d);
  assert.strictEqual((x.face_glyph_index(face, 0x1f600) >>> 0), 0xffffffff);

  // Render a glyph into the atlas
  const g = x.face_render_glyph(face, atlas, 0x41);
  assert.ok(g, 'face_render_glyph');
  const gv = new Uint32Array(instance.exports.memory.buffer, g, 6);
  assert.strictEqual(gv[0], 8); // width: 7 + 1 border
  assert.strictEqual(gv[1], 14); // height: ceil(12 + 0.5) + 1
  x.glyph_free(g);
  const sz = x.wasm_alloc(8);
  const dp = x.atlas_data(atlas, sz);
  const len = new Uint32Array(instance.exports.memory.buffer, sz, 1)[0];
  assert.strictEqual(len, 64 * 64);
  const atlasBytes = new Uint8Array(instance.exports.memory.buffer, dp, len);
  assert.ok(atlasBytes.some((b) => b === 255), 'atlas received glyph alpha');

  // Emoji face
  const ef = host.writeString('sans-serif');
  const eface = x.face_new(ef.ptr, ef.len, 14, 1);
  assert.ok(eface);
  assert.strictEqual((x.face_glyph_index(eface, 0x1f600) >>> 0), 0x1f600);
  assert.strictEqual((x.face_glyph_index(eface, 0x4e2d) >>> 0), 0xffffffff);

  // ---- shaper ----
  const sh = x.shaper_new(16);
  x.shaper_prepare(sh);
  // 'e' + U+0301, then 'x'
  x.shaper_add_codepoint(sh, 0x65, 0);
  x.shaper_add_codepoint(sh, 0x301, 0);
  x.shaper_add_codepoint(sh, 0x78, 1);
  const outp = x.wasm_alloc(16 * 8);
  const cells = x.shaper_shape(sh, face, outp, 16);
  assert.strictEqual(cells, 2);
  const ov = new Uint32Array(instance.exports.memory.buffer, outp, 4);
  assert.deepStrictEqual([ov[0], ov[1] >= 0x110000, ov[2], ov[3]], [0, true, 1, 0x78]);
  // Render the reserved grapheme glyph through the canvas
  const gg = x.face_render_glyph(face, atlas, ov[1]);
  assert.ok(gg);
  x.glyph_free(gg);
  assert.strictEqual(canvases[0].lastText.t, 'é');

  // shaper_test prints through a terminal and shapes its rows. The default
  // errors-only logger suppresses the info records for those shaped runs.
  const st = host.writeString('hello é');
  x.shaper_test(sh, face, st.ptr, st.len);
  assert.deepStrictEqual(logs, [], 'shaper info logs must be suppressed');
  assert.ok(x.shaper_shape(sh, face, outp, 16) > 0, 'shaper_test prepared a run');
  assert.strictEqual(new Uint32Array(instance.exports.memory.buffer, outp, 2)[1], 0x68);

  // ---- deferred face ----
  const d = x.deferred_face_new(f.ptr, f.len, 0);
  assert.ok(d);
  const before = canvases.length;
  x.deferred_face_load(d, 12);
  assert.strictEqual(canvases.length, before + 1);
  x.deferred_face_free(d);

  // ---- atlas debug canvas ----
  const h = x.atlas_debug_canvas(atlas);
  assert.ok(h > 0);
  const dc = zigjs.deleteValue(h);
  assert.strictEqual(dc.width, 64);
  assert.ok(dc.calls.some((c) => c[0] === 'putImageData'));

  // ---- cleanup / handle leaks ----
  x.shaper_free(sh);
  x.face_free(face);
  x.face_free(eface);
  x.atlas_free(atlas);
  assert.strictEqual(liveValues(zigjs), 0, 'leaked zig-js values');
  f.free();
  ef.free();
  st.free();
  console.log('ok');
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
