// Instantiates ghostty_wasm.wasm with stub JS imports and exercises the
// exported config / atlas / canvas face / shaper functions. Run by
// tests/node.rs: `node node_test.js <path-to-wasm>`.
const fs = require('fs');
const assert = require('assert');
const path = require('path');
const { createHost } = require(path.join(__dirname, '..', 'js', 'ghostty_host.js'));

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
      clearRect: (...a) => self.calls.push(['clearRect', ...a]),
      fillRect: (...a) => self.calls.push(['fillRect', ...a]),
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
      putImageData: (...a) => self.calls.push(['putImageData', ...a]),
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
const fakeGlobal = {
  document: {
    createElement: (tag) => {
      assert.strictEqual(tag, 'canvas');
      const c = new FakeCanvas();
      canvases.push(c);
      return c;
    },
    getElementById: () => null,
  },
  ImageData: FakeImageData,
};

(async () => {
  const logs = [];
  const host = createHost({ global: fakeGlobal, log: (s) => logs.push(s) });
  const bytes = fs.readFileSync(process.argv[2]);
  const { instance } = await WebAssembly.instantiate(bytes, host.imports);
  host.setInstance(instance);
  const x = instance.exports;
  x.ghostty_wasm_init();

  // ---- config ----
  {
    const cfg = x.config_new();
    assert.ok(cfg);
    const s = host.writeString('font-size = 18\nwindow-padding-x = 7\nnot-a-key = 1\n');
    x.config_load_string(cfg, s.ptr, s.len);
    s.free();
    assert.strictEqual(x.config_diagnostics_count(cfg), 1);
    x.config_finalize(cfg);
    const cap = 1 << 16;
    const out = x.wasm_alloc(cap);
    const n = x.config_format(cfg, out, cap);
    assert.ok(n > 0 && n <= cap);
    const text = host.readString(out, n);
    x.wasm_free(out, cap);
    assert.match(text, /font-size = 18/);
    assert.match(text, /window-padding-x = 7/);
    x.config_free(cfg);
  }

  // ---- atlas ----
  const atlas = x.atlas_new(64, 0); // grayscale
  assert.ok(atlas);
  assert.strictEqual(x.atlas_size(atlas), 64);

  // ---- face ----
  const f = host.writeString('monospace');
  const face = x.face_new(f.ptr, f.len, 12, 0);
  assert.ok(face, 'face_new');
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

  // shaper_test prints through a terminal and logs the shaped runs
  const st = host.writeString('hello é');
  x.shaper_test(sh, face, st.ptr, st.len);
  assert.ok(logs.some((l) => l.startsWith('info(')), logs.join('\n'));

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
  const dc = host.value(h);
  assert.strictEqual(dc.width, 64);
  assert.ok(dc.calls.some((c) => c[0] === 'putImageData'));

  // ---- cleanup / handle leaks ----
  x.shaper_free(sh);
  x.face_free(face);
  x.face_free(eface);
  x.atlas_free(atlas);
  f.free();
  ef.free();
  st.free();
  console.log('ok');
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
