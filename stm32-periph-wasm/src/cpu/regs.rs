#[derive(Clone, Copy, Debug)]
pub struct Regs {
    pub r: [u32; 16],
    pub xpsr: u32,
    pub primask: u32,
    pub control: u32,
}

impl Regs {
    pub fn new(sp: u32, pc: u32) -> Self {
        let mut r = [0u32; 16];
        r[13] = sp;
        r[14] = 0xFFFFFFFD;
        r[15] = pc | 1;
        Self { r, xpsr: 0x01000000, primask: 0, control: 0 }
    }
}
