// Firmware image loaders: Intel HEX, ELF32, and GCC linker map parsers.
// Pure JS, no imports. All functions are synchronous.

// ── Intel HEX ──────────────────────────────────────────────────────────────
// Returns { flash, ram, entry } — Uint8Array images for 0x08000000 and
// 0x20000000, plus an optional entry point from the 0x05 record.
export function parseIntelHex(text) {
    const image = new Map(); // addr -> byte
    let base = 0;            // extended linear address (record 0x04)
    let entry = null;
    for (let line of text.split(/\r?\n/)) {
        line = line.trim();
        if (!line) continue;
        if (line[0] !== ':') throw new Error('not an Intel HEX record: ' + line.slice(0, 16));
        const bytes = [];
        for (let i = 1; i < line.length; i += 2) bytes.push(parseInt(line.slice(i, i + 2), 16));
        const count = bytes[0], addr = (bytes[1] << 8) | bytes[2], type = bytes[3];
        let sum = 0;
        for (const b of bytes) sum = (sum + b) & 0xFF;
        if (sum !== 0) throw new Error('bad HEX checksum on line: ' + line.slice(0, 16));
        const data = bytes.slice(4, 4 + count);
        if (type === 0x00) {
            for (let i = 0; i < count; i++) image.set(base + addr + i, data[i]);
        } else if (type === 0x01) {
            break;
        } else if (type === 0x04) {
            base = ((data[0] << 8) | data[1]) << 16;
        } else if (type === 0x05) {
            entry = (data[0] << 24) | (data[1] << 16) | (data[2] << 8) | data[3];
        }
        // types 0x02/0x03 (segment) are rare; ignore
    }
    const toImage = (start, end) => {
        const out = new Uint8Array(end - start);
        let any = false;
        for (const [addr, b] of image) {
            if (addr >= start && addr < end) { out[addr - start] = b; any = true; }
        }
        return any ? out : null;
    };
    return { flash: toImage(0x08000000, 0x08100000), ram: toImage(0x20000000, 0x20020000), entry };
}

// ── ELF32 ─────────────────────────────────────────────────────────────────
// Extracts PT_LOAD segments (flash/RAM images + preload list) and, if the
// symtab is present, the symbol table for the symbols panel.
export function parseElf(bytes) {
    if (bytes.length < 52) throw new Error('file too small to be an ELF');
    const b = new Uint8Array(bytes);
    if (b[0] !== 0x7F || b[1] !== 0x45 || b[2] !== 0x4C || b[3] !== 0x46) throw new Error('not an ELF file');
    if (b[4] !== 1 || b[5] !== 1) throw new Error('only 32-bit little-endian ELF is supported');
    const u32 = (o) => (b[o] | (b[o + 1] << 8) | (b[o + 2] << 16) | (b[o + 3] << 24)) >>> 0;
    const u16 = (o) => (b[o] | (b[o + 1] << 8)) >>> 0;
    const entry = u32(24);
    const phoff = u32(28), shentsize = u16(46), shnum = u16(48), shstrndx = u16(50);
    const phentsize = u16(42), phnum = u16(44);

    const segments = [];
    for (let i = 0; i < phnum; i++) {
        const o = phoff + i * phentsize;
        if (u32(o) !== 1) continue; // PT_LOAD
        const off = u32(o + 4), vaddr = u32(o + 8), filesz = u32(o + 16), memsz = u32(o + 20);
        if (!filesz) continue;
        segments.push({ vaddr, data: new Uint8Array(b.slice(off, off + filesz)), memsz });
    }
    if (segments.length === 0) throw new Error('ELF has no loadable segments');

    const FLASH_START = 0x08000000, FLASH_END = 0x08100000;
    const RAM_START = 0x20000000, RAM_END = 0x20020000;
    const flash = new Uint8Array(FLASH_END - FLASH_START);
    const ram = new Uint8Array(RAM_END - RAM_START);
    const extraMem = [];
    for (const seg of segments) {
        if (seg.vaddr >= FLASH_START && seg.vaddr + seg.data.length <= FLASH_END) {
            flash.set(seg.data, seg.vaddr - FLASH_START);
        } else if (seg.vaddr >= RAM_START && seg.vaddr + seg.data.length <= RAM_END) {
            ram.set(seg.data, seg.vaddr - RAM_START);
            extraMem.push({ addr: seg.vaddr, data: seg.data });
        } else {
            throw new Error('loadable segment at 0x' + seg.vaddr.toString(16) + ' outside FLASH/RAM');
        }
    }

    let symbols = null;
    if (shentsize && shnum) {
        const shoff = u32(32);
        let symtab = null, strtab = null;
        for (let i = 0; i < shnum; i++) {
            const o = shoff + i * shentsize;
            if (o + shentsize > b.length) break;
            const type = u32(o + 4);
            if (type === 2) symtab = { offset: u32(o + 16), size: u32(o + 20), entsize: u32(o + 36), link: u16(o + 24) };
            if (type === 3 && strtab === null) strtab = { offset: u32(o + 16), size: u32(o + 20) };
        }
        if (symtab && strtab && symtab.entsize) {
            symbols = [];
            const names = new Uint8Array(b.slice(strtab.offset, strtab.offset + strtab.size));
            const count = Math.floor(symtab.size / symtab.entsize);
            for (let i = 0; i < count; i++) {
                const o = symtab.offset + i * symtab.entsize;
                const nameOff = u32(o), value = u32(o + 4), size = u32(o + 8);
                const info = b[o + 12], shndx = u16(o + 14);
                if (!nameOff || shndx === 0) continue; // no name or UNDEF
                const type = info & 0xF;
                if (type !== 2 && type !== 1) continue; // funcs + objects only
                let name = '';
                for (let j = nameOff; j < names.length && names[j] !== 0; j++) name += String.fromCharCode(names[j]);
                symbols.push({ name, addr: value, size });
            }
            symbols.sort((a, c) => a.addr - c.addr);
        }
    }
    const any = (img) => img.some((v) => v !== 0);
    return { flash: any(flash) ? flash : null, ram: any(ram) ? ram : null, extraMem, entry, symbols };
}

// ── GCC linker map ─────────────────────────────────────────────────────────
// Extracts "0x08000185                _start" style symbol lines.
export function parseMap(text) {
    const symbols = [];
    const seen = new Set();
    for (const line of text.split(/\r?\n/)) {
        const m = line.match(/0x([0-9a-fA-F]{8,})\s+([_a-zA-Z][_a-zA-Z0-9$]*)/);
        if (!m) continue;
        const addr = parseInt(m[1], 16);
        if (!((addr >= 0x08000000 && addr < 0x08100000) || (addr >= 0x20000000 && addr < 0x20020000))) continue;
        if (seen.has(m[2])) continue;
        seen.add(m[2]);
        symbols.push({ name: m[2], addr });
    }
    symbols.sort((a, b) => a.addr - b.addr);
    return symbols;
}
