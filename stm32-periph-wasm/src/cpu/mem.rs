pub trait Memory {
    fn read8(&self, addr: u32) -> u8;
    fn read16(&self, addr: u32) -> u16;
    fn read32(&self, addr: u32) -> u32;
    fn write8(&mut self, addr: u32, v: u8);
    fn write16(&mut self, addr: u32, v: u16);
    fn write32(&mut self, addr: u32, v: u32);
}

pub struct FlatMemory {
    pub flash: Vec<u8>,
    pub ram: Vec<u8>,
    pub flash_base: u32,
    pub ram_base: u32,
}

impl FlatMemory {
    pub fn new(flash_size: usize, ram_size: usize) -> Self {
        Self { flash: vec![0; flash_size], ram: vec![0; ram_size], flash_base: 0x08000000, ram_base: 0x20000000 }
    }
}

fn is_periph(addr: u32) -> bool {
    (addr >= 0x40000000 && addr < 0x51000000) || (addr >= 0x60000000 && addr < 0x62000000) || (addr >= 0xA0000000 && addr < 0xA2000000) || (addr >= 0xE0000000 && addr < 0xE1000000)
}

impl Memory for FlatMemory {
    fn read8(&self, addr: u32) -> u8 {
        if is_periph(addr) {
            let v = crate::sys().p.read(crate::sys(), addr & !3, 4);
            return ((v >> ((addr & 3) * 8)) & 0xFF) as u8;
        }
        if addr >= self.flash_base && addr < self.flash_base + self.flash.len() as u32 {
            self.flash[(addr - self.flash_base) as usize]
        } else if addr >= self.ram_base && addr < self.ram_base + self.ram.len() as u32 {
            self.ram[(addr - self.ram_base) as usize]
        } else { 0 }
    }
    fn read16(&self, addr: u32) -> u16 { let lo = self.read8(addr) as u16; let hi = self.read8(addr+1) as u16; lo | (hi<<8) }
    fn read32(&self, addr: u32) -> u32 { let b0=self.read8(addr) as u32; let b1=self.read8(addr+1) as u32; let b2=self.read8(addr+2) as u32; let b3=self.read8(addr+3) as u32; b0|(b1<<8)|(b2<<16)|(b3<<24) }
    fn write8(&mut self, addr: u32, v: u8) {
        if is_periph(addr) {
            let al = addr & !3; let sh = (addr & 3)*8; let cur = crate::sys().p.read(crate::sys(), al, 4); let val = (cur & !(0xFFu32<<sh)) | ((v as u32)<<sh);
            crate::sys().p.write(crate::sys(), al, 4, val); return;
        }
        if addr >= self.flash_base && addr < self.flash_base + self.flash.len() as u32 { } else if addr >= self.ram_base && addr < self.ram_base + self.ram.len() as u32 { self.ram[(addr-self.ram_base) as usize]=v; }
    }
    fn write16(&mut self, addr: u32, v: u16) { self.write8(addr, (v&0xFF) as u8); self.write8(addr+1, (v>>8) as u8); }
    fn write32(&mut self, addr: u32, v: u32) { self.write8(addr, (v&0xFF)as u8); self.write8(addr+1, ((v>>8)&0xFF)as u8); self.write8(addr+2, ((v>>16)&0xFF)as u8); self.write8(addr+3, ((v>>24)&0xFF)as u8); }
}
