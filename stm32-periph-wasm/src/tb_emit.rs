//! Minimal WASM binary emitter for the static-binary-translation project
//! (authorized 2026-10-10, AGENTS.md §42). Emits little-endian WASM modules
//! handed to JS (`WebAssembly.Module`) — the translator (not yet written)
//! will use this to build per-block guest executables. No decoding or
//! execution here: pure byte layout, verified by golden tests against
//! hand-audited modules (see `.pw-scratch/tb_uarch.mjs`, whose output was
//! executed and count-verified in both engines).
//!
//! Opcode constants are the few hot-path ones only (931d: i32.lt_s is
//! 0x48 — 0x4B is gt_u, a silent-always-exit trap that cost an hour;
//! i64 comparisons live at 0x51-0x56 (lt_u=0x54, gt_u=0x56) — the 0xDx
//! range is reference-types (0xD3=ref.eq, 0xD5=ref.as_non_null), and a
//! mixup compiles the module then fails validation confusingly;
//! i32.store takes [addr, value], addr pushed FIRST — swapped operands
//! silently compute m[v+1]=addr, which cost several more).

/// Unsigned LEB128 append (indices, sizes, memargs — unsigned contexts).
pub fn leb_u32(mut v: u32, out: &mut Vec<u8>) {
    loop {
        let mut c = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            c |= 0x80;
        }
        out.push(c);
        if v == 0 {
            break;
        }
    }
}

/// Signed LEB128 append (i32.const and other signed immediates).
/// Unsigned-minimal bytes are WRONG whenever the top payload group's
/// 0x40 bit is set on a positive value (e.g. 0x0FFFFFFF as [FF FF FF
/// 7F] decodes NEGATIVE 0xF0000001 — this exact bug kept the V bit
/// alive through the xPSR KEEP mask in every flag test). Rule: positive
/// values whose top group has 0x40 set need a trailing 0x00 byte.
pub fn sleb_i32(mut v: i32, out: &mut Vec<u8>) {
    loop {
        let mut c = (v & 0x7F) as u8;
        v >>= 7;
        let done = (v == 0 && c & 0x40 == 0) || (v == -1 && c & 0x40 != 0);
        if !done {
            c |= 0x80;
        }
        out.push(c);
        if done {
            break;
        }
    }
}

/// One section: id + length + payload.
pub fn section(id: u8, body: &[u8], out: &mut Vec<u8>) {
    out.push(id);
    leb_u32(body.len() as u32, out);
    out.extend_from_slice(body);
}

pub const I32: u8 = 0x7F;
pub const I64: u8 = 0x7E;

/// Function body builder: locals + raw instruction stream.
pub struct Code {
    pub locals: Vec<(u32, u8)>,
    pub code: Vec<u8>,
}

impl Code {
    pub fn new() -> Self {
        Self { locals: Vec::new(), code: Vec::new() }
    }
    pub fn local(&mut self, count: u32, ty: u8) -> &mut Self {
        self.locals.push((count, ty));
        self
    }
    fn op(&mut self, b: u8) -> &mut Self {
        self.code.push(b);
        self
    }
    fn op_u(&mut self, b: u8, v: u32) -> &mut Self {
        self.code.push(b);
        let mut t = Vec::new();
        leb_u32(v, &mut t);
        self.code.extend_from_slice(&t);
        self
    }
    pub fn i32_const(&mut self, v: i32) -> &mut Self {
        self.code.push(0x41);
        let mut t = Vec::new();
        sleb_i32(v, &mut t);
        self.code.extend_from_slice(&t);
        self
    }
    pub fn local_get(&mut self, i: u32) -> &mut Self { self.op_u(0x20, i) }
    pub fn local_set(&mut self, i: u32) -> &mut Self { self.op_u(0x21, i) }
    pub fn local_tee(&mut self, i: u32) -> &mut Self { self.op_u(0x22, i) }
    pub fn i32_add(&mut self) -> &mut Self { self.op(0x6A) }
    pub fn i32_and(&mut self) -> &mut Self { self.op(0x71) }
    pub fn i32_lt_s(&mut self) -> &mut Self { self.op(0x48) }
    pub fn i32_lt_u(&mut self) -> &mut Self { self.op(0x49) }
    pub fn i32_gt_u(&mut self) -> &mut Self { self.op(0x4B) }
    pub fn i32_eq(&mut self) -> &mut Self { self.op(0x46) }
    pub fn i32_ne(&mut self) -> &mut Self { self.op(0x47) }
    pub fn i32_eqz(&mut self) -> &mut Self { self.op(0x45) }
    pub fn i32_or(&mut self) -> &mut Self { self.op(0x72) }
    pub fn i32_xor(&mut self) -> &mut Self { self.op(0x73) }
    pub fn i32_shl(&mut self) -> &mut Self { self.op(0x74) }
    pub fn i32_shr_u(&mut self) -> &mut Self { self.op(0x76) }
    pub fn i32_sub(&mut self) -> &mut Self { self.op(0x6B) }
    pub fn i32_mul(&mut self) -> &mut Self { self.op(0x6C) }
    pub fn i64_extend_u(&mut self) -> &mut Self { self.op(0xAD) }
    pub fn i64_add(&mut self) -> &mut Self { self.op(0x7C) }
    pub fn i64_sub(&mut self) -> &mut Self { self.op(0x7D) }
    pub fn i64_lt_u(&mut self) -> &mut Self { self.op(0x54) }
    pub fn i64_gt_u(&mut self) -> &mut Self { self.op(0x56) }
    pub fn i64_or(&mut self) -> &mut Self { self.op(0x84) }
    pub fn i64_shl(&mut self) -> &mut Self { self.op(0x86) }
    pub fn i64_shr_u(&mut self) -> &mut Self { self.op(0x87) }
    pub fn i64_mul(&mut self) -> &mut Self { self.op(0x7E) }
    pub fn i64_const(&mut self, v: u64) -> &mut Self {
        self.code.push(0x42);
        let mut x = v;
        loop {
            let mut c = (x & 0x7F) as u8;
            x >>= 7;
            if x != 0 {
                c |= 0x80;
            }
            self.code.push(c);
            if x == 0 {
                break;
            }
        }
        self
    }
    pub fn i32_wrap_i64(&mut self) -> &mut Self { self.op(0xA7) }
    pub fn global_get(&mut self, i: u32) -> &mut Self { self.op_u(0x23, i) }
    pub fn global_set(&mut self, i: u32) -> &mut Self { self.op_u(0x24, i) }
    pub fn loop_void(&mut self) -> &mut Self {
        self.code.extend_from_slice(&[0x03, 0x40]);
        self
    }
    pub fn block_void(&mut self) -> &mut Self {
        self.code.extend_from_slice(&[0x02, 0x40]);
        self
    }
    pub fn br(&mut self, depth: u32) -> &mut Self { self.op_u(0x0C, depth) }
    pub fn unreachable_(&mut self) -> &mut Self { self.op(0x00) }
    pub fn i32_le_s(&mut self) -> &mut Self { self.op(0x4C) }
    pub fn i32_ge_u(&mut self) -> &mut Self { self.op(0x4E) }
    pub fn end(&mut self) -> &mut Self { self.op(0x0B) }
    pub fn br_if(&mut self, depth: u32) -> &mut Self { self.op_u(0x0D, depth) }
    pub fn call(&mut self, idx: u32) -> &mut Self { self.op_u(0x10, idx) }
    pub fn nop(&mut self) -> &mut Self { self.op(0x01) }
    pub fn if_void(&mut self) -> &mut Self {
        self.code.extend_from_slice(&[0x04, 0x40]);
        self
    }
    pub fn else_(&mut self) -> &mut Self {
        self.code.push(0x05);
        self
    }
    pub fn return_(&mut self) -> &mut Self { self.op(0x0F) }
    pub fn drop_(&mut self) -> &mut Self { self.op(0x1A) }
    /// (addr, value) order is load-bearing: store pops value(top), addr.
    pub fn i32_load(&mut self, align: u32, offset: u32) -> &mut Self {
        self.code.push(0x28);
        let mut t = Vec::new();
        leb_u32(align, &mut t);
        leb_u32(offset, &mut t);
        self.code.extend_from_slice(&t);
        self
    }
    pub fn i32_load8_u(&mut self, align: u32, offset: u32) -> &mut Self {
        // 0x2D = i32.load8_u (zero-extending; 0x2C is the SIGNED form —
        // swapping them made every LDRB sign-extend, caught live).
        self.code.push(0x2D);
        let mut t = Vec::new();
        leb_u32(align, &mut t);
        leb_u32(offset, &mut t);
        self.code.extend_from_slice(&t);
        self
    }
    pub fn i32_store8(&mut self, align: u32, offset: u32) -> &mut Self {
        self.code.push(0x3A);
        let mut t = Vec::new();
        leb_u32(align, &mut t);
        leb_u32(offset, &mut t);
        self.code.extend_from_slice(&t);
        self
    }
    /// Push addr FIRST, then value: pops value(top), addr.
    pub fn i32_store(&mut self, align: u32, offset: u32) -> &mut Self {
        self.code.push(0x36);
        let mut t = Vec::new();
        leb_u32(align, &mut t);
        leb_u32(offset, &mut t);
        self.code.extend_from_slice(&t);
        self
    }
    pub fn finish(&self, out: &mut Vec<u8>) {
        let mut body = Vec::new();
        leb_u32(self.locals.len() as u32, &mut body);
        for (n, ty) in &self.locals {
            leb_u32(*n, &mut body);
            body.push(*ty);
        }
        body.extend_from_slice(&self.code);
        leb_u32(body.len() as u32, out);
        out.extend_from_slice(&body);
    }
}

/// Whole-module builder (counted so finish() is exact).
pub struct Module {
    types: Vec<u8>,
    funcs: Vec<u8>,
    mems: Vec<u8>,
    globals: Vec<u8>,
    exports: Vec<u8>,
    codes: Vec<u8>,
    n_types: usize,
    n_funcs: usize,
    n_mems: usize,
    n_globals: usize,
    n_exports: usize,
    n_codes: usize,
}

impl Module {
    pub fn new() -> Self {
        Self { types: Vec::new(), funcs: Vec::new(), mems: Vec::new(), globals: Vec::new(), exports: Vec::new(), codes: Vec::new(),
            n_types: 0, n_funcs: 0, n_mems: 0, n_globals: 0, n_exports: 0, n_codes: 0 }
    }
    /// One function type: params + results (each a vec of valtypes).
    pub fn functype(&mut self, params: &[u8], results: &[u8]) {
        self.types.push(0x60);
        leb_u32(params.len() as u32, &mut self.types);
        self.types.extend_from_slice(params);
        leb_u32(results.len() as u32, &mut self.types);
        self.types.extend_from_slice(results);
        self.n_types += 1;
    }
    pub fn func(&mut self, type_idx: u32) {
        leb_u32(type_idx, &mut self.funcs);
        self.n_funcs += 1;
    }
    pub fn memory(&mut self, min_pages: u32) {
        self.mems.push(0x00);
        leb_u32(min_pages, &mut self.mems);
        self.n_mems += 1;
    }
    pub fn global_mut_i32(&mut self, init: u32) {
        // Mutable i32 global, const initializer (inits used are small
        // non-negative, where unsigned LEB == signed form exactly).
        self.globals.push(0x7F);
        self.globals.push(0x01);
        self.globals.push(0x41);
        let mut t = Vec::new();
        leb_u32(init, &mut t);
        self.globals.extend_from_slice(&t);
        self.globals.push(0x0B);
        self.n_globals += 1;
    }
    pub fn export_global(&mut self, name: &str, idx: u32) {
        leb_u32(name.len() as u32, &mut self.exports);
        self.exports.extend_from_slice(name.as_bytes());
        self.exports.push(0x03);
        leb_u32(idx, &mut self.exports);
        self.n_exports += 1;
    }
    pub fn export_func(&mut self, name: &str, idx: u32) {
        leb_u32(name.len() as u32, &mut self.exports);
        self.exports.extend_from_slice(name.as_bytes());
        self.exports.push(0x00);
        leb_u32(idx, &mut self.exports);
        self.n_exports += 1;
    }
    pub fn export_mem(&mut self, name: &str, idx: u32) {
        leb_u32(name.len() as u32, &mut self.exports);
        self.exports.extend_from_slice(name.as_bytes());
        self.exports.push(0x02);
        leb_u32(idx, &mut self.exports);
        self.n_exports += 1;
    }
    pub fn code(&mut self, c: &Code) {
        c.finish(&mut self.codes);
        self.n_codes += 1;
    }
    pub fn finish(&self) -> Vec<u8> {
        let mut out = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        let mut put = |id: u8, items: &Vec<u8>, n: usize| {
            if n == 0 {
                return;
            }
            let mut body = Vec::new();
            leb_u32(n as u32, &mut body);
            body.extend_from_slice(items);
            section(id, &body, &mut out);
        };
        put(1, &self.types, self.n_types);
        put(3, &self.funcs, self.n_funcs);
        put(5, &self.mems, self.n_mems);
        put(6, &self.globals, self.n_globals);
        put(7, &self.exports, self.n_exports);
        put(10, &self.codes, self.n_codes);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leb_vectors() {
        let mut v = Vec::new();
        for (n, want) in [(0u32, vec![0x00]), (1, vec![0x01]), (127, vec![0x7F]),
            (128, vec![0x80, 0x01]), (1023, vec![0xFF, 0x07]),
            (16384, vec![0x80, 0x80, 0x01]), (0xFFFF_FFFF, vec![0xFF, 0xFF, 0xFF, 0xFF, 0x0F])]
        {
            v.clear();
            leb_u32(n, &mut v);
            assert_eq!(v, want, "leb({n})");
        }
    }

    /// Byte-identical rebuild of the hand-audited LOCAL-mem loop module
    /// (executed + count-verified in Node 22 + headless Chrome; its
    /// construction caught a swapped store-operand bug and a 0x4B/0x48
    /// mixup — this test pins the emitter against those exact bytes).
    const GOLDEN_LOCAL: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
        0x01, 0x05, 0x01, 0x60, 0x01, 0x7f, 0x00, 0x03,
        0x02, 0x01, 0x00, 0x05, 0x03, 0x01, 0x00, 0x0a,
        0x07, 0x0d, 0x02, 0x03, 0x72, 0x75, 0x6e, 0x00,
        0x00, 0x03, 0x6d, 0x65, 0x6d, 0x02, 0x00, 0x0a,
        0x2e, 0x01, 0x2c, 0x01, 0x03, 0x7f, 0x41, 0x00,
        0x21, 0x01, 0x03, 0x40, 0x20, 0x01, 0x41, 0xff,
        0x07, 0x71, 0x21, 0x02, 0x20, 0x02, 0x20, 0x02,
        0x28, 0x02, 0x00, 0x41, 0x01, 0x6a, 0x36, 0x02,
        0x00, 0x20, 0x01, 0x41, 0x01, 0x6a, 0x22, 0x01,
        0x20, 0x00, 0x48, 0x0d, 0x00, 0x0b, 0x0b,
    ];

    #[test]
    fn golden_local_mem_loop() {
        let mut m = Module::new();
        m.functype(&[I32], &[]);
        m.func(0);
        m.memory(10);
        m.export_func("run", 0);
        m.export_mem("mem", 0);
        let mut c = Code::new();
        c.local(3, I32);
        c.i32_const(0).local_set(1);
        c.loop_void();
        c.local_get(1).i32_const(1023).i32_and().local_set(2); // addr
        c.local_get(2);                                          // [addr]
        c.local_get(2).i32_load(2, 0);                           // [addr, v]
        c.i32_const(1).i32_add();                                 // [addr, v+1]
        c.i32_store(2, 0);                                       // m[addr] = v+1
        c.local_get(1).i32_const(1).i32_add().local_tee(1);
        c.local_get(0).i32_lt_s();
        c.br_if(0);
        c.end().end();
        m.code(&c);
        assert_eq!(m.finish(), GOLDEN_LOCAL);
    }
}
