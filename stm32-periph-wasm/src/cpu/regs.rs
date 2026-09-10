#[derive(Clone, Copy, Debug)]
pub struct Regs {
    pub r: [u32; 16],
    pub xpsr: u32,
    pub primask: u32,
    /// BASEPRI mask value, raw as written (firmware writes its shifted
    /// priority, e.g. 0x50 for 4-bit fields). 0 = mask disabled. Compared
    /// raw against raw priority bytes, which orders identically.
    pub basepri: u8,
    /// FAULTMASK: when set, only NMI can activate.
    pub faultmask: bool,
    pub control: u32,
    /// Banked stacks. `r[13]` always mirrors the CURRENT SP (MSP in handler
    /// mode; MSP or PSP per CONTROL.SPSEL in thread mode). FreeRTOS switches
    /// tasks by writing PSP via MRS/MSR while in handler mode.
    pub msp: u32,
    pub psp: u32,
    /// VFPv4-SP single-precision file S0-S31 (raw f32 bits; Dd aliases
    /// S(2d)/S(2d+1)). Reset to 0 (hardware UNKNOWN; zero is the sane pick).
    pub s: [u32; 32],
    /// FPSCR (NZCV/QC/DN/FZ/RMode/exception flags). Reset 0.
    pub fpscr: u32,
}

impl Regs {
    pub fn new(sp: u32, pc: u32) -> Self {
        let mut r = [0u32; 16];
        r[13] = sp;
        r[14] = 0xFFFFFFFD;
        r[15] = pc | 1;
        Self { r, xpsr: 0x01000000, primask: 0, basepri: 0, faultmask: false, control: 0, msp: sp, psp: 0, s: [0u32; 32], fpscr: 0 }
    }
}
