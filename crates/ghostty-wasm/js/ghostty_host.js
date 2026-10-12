// Host-side glue for the `ghostty_js` imports (see
// crates/ghostty-os/src/wasm/js.rs for the protocol) and `env.log`.
// Usable from a browser (ES module) or node (CommonJS).
//
//   const host = createHost({ globalThis, log });
//   const { instance } = await WebAssembly.instantiate(bytes, host.imports);
//   host.setInstance(instance);

function createHost({ global = globalThis, log = console.log } = {}) {
  const dec = new TextDecoder();
  const enc = new TextEncoder();
  // Handle table. Handle 0 is reserved for "invalid / threw".
  const table = [undefined];
  const free = [];
  let instance = null;

  const put = (v) => {
    if (free.length > 0) {
      const h = free.pop();
      table[h] = v;
      return h;
    }
    table.push(v);
    return table.length - 1;
  };
  const mem = () => new Uint8Array(instance.exports.memory.buffer);
  const str = (ptr, len) => dec.decode(mem().subarray(ptr, ptr + len));
  const guard = (f) => {
    try {
      return f();
    } catch (e) {
      host.lastError = e;
      return 0;
    }
  };

  const ghostty_js = {
    global: () => put(global),
    get: (obj, ptr, len) => guard(() => put(table[obj][str(ptr, len)])),
    set: (obj, ptr, len, val) =>
      guard(() => {
        table[obj][str(ptr, len)] = table[val];
        return 1;
      }),
    call: (obj, ptr, len, argsPtr, argc) =>
      guard(() => {
        const args = Array.from(new Uint32Array(instance.exports.memory.buffer, argsPtr, argc), (h) => table[h]);
        const o = table[obj];
        return put(o[str(ptr, len)](...args));
      }),
    new: (ctor, argsPtr, argc) =>
      guard(() => {
        const args = Array.from(new Uint32Array(instance.exports.memory.buffer, argsPtr, argc), (h) => table[h]);
        return put(new table[ctor](...args));
      }),
    string: (ptr, len) => put(str(ptr, len)),
    number: (v) => put(v),
    boolean: (v) => put(v !== 0),
    to_f64: (h) => Number(table[h]),
    string_len: (h) => enc.encode(String(table[h])).length,
    string_copy: (h, ptr, cap) => {
      const bytes = enc.encode(String(table[h]));
      const n = Math.min(bytes.length, cap);
      mem().set(bytes.subarray(0, n), ptr);
      return n;
    },
    is_nullish: (h) => (table[h] === undefined || table[h] === null ? 1 : 0),
    bytes: (ptr, len) => put(new Uint8ClampedArray(mem().slice(ptr, ptr + len).buffer)),
    copy_bytes: (h, dst, len) =>
      guard(() => {
        const src = table[h];
        if (src.length !== len) throw new Error(`length mismatch ${src.length} != ${len}`);
        mem().set(new Uint8Array(src.buffer, src.byteOffset, len), dst);
        return 1;
      }),
    release: (h) => {
      if (h > 0) {
        table[h] = undefined;
        free.push(h);
      }
    },
  };

  const host = {
    lastError: null,
    imports: { ghostty_js, env: { log: (ptr, len) => log(str(ptr, len)) } },
    setInstance(i) {
      instance = i;
    },
    // Helpers for callers of the exports.
    writeString(s) {
      const bytes = enc.encode(s);
      const ptr = instance.exports.wasm_alloc(bytes.length);
      mem().set(bytes, ptr);
      return { ptr, len: bytes.length, free: () => instance.exports.wasm_free(ptr, bytes.length) };
    },
    readString(ptr, len) {
      return str(ptr, len);
    },
    value: (h) => table[h],
    handleCount: () => table.length - 1 - free.length,
  };
  return host;
}

if (typeof module !== 'undefined') module.exports = { createHost };
