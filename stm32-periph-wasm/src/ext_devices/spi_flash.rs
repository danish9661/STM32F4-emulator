use std::collections::VecDeque;
use crate::system::System;
use super::ExtDevice;

pub struct SpiFlashConfig {
    pub peripheral: String,
    pub jedec_id: u32,
    pub content: Vec<u8>,
    pub size: usize,
    pub cs: Option<String>,
}

pub struct SpiFlash {
    pub config: SpiFlashConfig,
    name: String,
    pub reply: Option<Reply>,
    cmd: Option<(Command, Vec<u8>)>,
    pub wel: bool,
    pub status1: u8,
    pub pending_program: Option<Program>,
    pending_erase: Option<(usize, usize)>, // (start, len) in content
    pub cs_state: bool,
    /// MISO bytes that return 0 while the command/address bytes are clocked in
    /// (real SPI flash gates the first data out only after the full command).
    pub dummy_pending: u8,
}

pub struct Program {
    pub addr: usize,
    pub data: Vec<u8>,
}

pub enum Reply {
    Data(VecDeque<u8>),
    FileContent(usize),
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    WriteEnable = 0x06,
    WriteDisable = 0x04,
    ReadId = 0x9F,
    ReadDeviceId = 0x90,
    ReadStatus1 = 0x05,
    ReadStatus2 = 0x35,
    WriteStatus1 = 0x01,
    WriteStatus2 = 0x31,
    PageProgram = 0x02,
    QuadPageProgram = 0x32,
    ReadData = 0x03,
    FastRead = 0x0B,
    SectorErase4k = 0x20,
    BlockErase32k = 0x52,
    BlockErase64k = 0xD8,
    ChipErase = 0xC7,
    DeepPowerDown = 0xB9,
    ReleasePowerDown = 0xAB,
}

impl Command {
    /// Number of fixed argument bytes that follow the opcode.
    /// `None` = address + variable-length data phase (terminated by CS deassert).
    fn arg_count(self) -> Option<usize> {
        use Command::*;
        match self {
            WriteEnable | WriteDisable | ReadId | ReadStatus1 | ReadStatus2 | ChipErase
            | DeepPowerDown | ReleasePowerDown => Some(0),
            WriteStatus1 | WriteStatus2 => Some(1),
            ReadData | SectorErase4k | BlockErase32k | BlockErase64k => Some(3),
            FastRead | ReadDeviceId | QuadPageProgram => Some(4),
            PageProgram => Some(3),
        }
    }
}

impl TryFrom<u8> for Command {
    type Error = ();
    fn try_from(v: u8) -> Result<Self, ()> {
        use Command::*;
        match v {
            0x06 => Ok(WriteEnable),
            0x04 => Ok(WriteDisable),
            0x9F => Ok(ReadId),
            0x90 => Ok(ReadDeviceId),
            0x05 => Ok(ReadStatus1),
            0x35 => Ok(ReadStatus2),
            0x01 => Ok(WriteStatus1),
            0x31 => Ok(WriteStatus2),
            0x02 => Ok(PageProgram),
            0x32 => Ok(QuadPageProgram),
            0x03 => Ok(ReadData),
            0x0B => Ok(FastRead),
            0x20 => Ok(SectorErase4k),
            0x52 => Ok(BlockErase32k),
            0xD8 => Ok(BlockErase64k),
            0xC7 => Ok(ChipErase),
            0xB9 => Ok(DeepPowerDown),
            0xAB => Ok(ReleasePowerDown),
            _ => Err(()),
        }
    }
}

impl SpiFlash {
    pub fn new(config: SpiFlashConfig) -> Self {
        SpiFlash {
            config,
            name: String::new(),
            reply: None,
            cmd: None,
            wel: false,
            status1: 0,
            pending_program: None,
            pending_erase: None,
            cs_state: true,
            dummy_pending: 0,
        }
    }

    fn addr24(args: &[u8]) -> usize {
        ((args[0] as usize) << 16) | ((args[1] as usize) << 8) | args[2] as usize
    }

    /// Commit a buffered page-program or erase into the content buffer.
    fn commit_pending(&mut self) {
        let mut did_op = false;
        if let Some(p) = self.pending_program.take() {
            for (i, b) in p.data.iter().enumerate() {
                let idx = (p.addr + i) % self.config.size;
                self.config.content[idx] &= b; // program ANDs bits (1 -> 0)
            }
            did_op = true;
        }
        if let Some((start, len)) = self.pending_erase.take() {
            for i in 0..len {
                let idx = (start + i) % self.config.size;
                self.config.content[idx] = 0xFF;
            }
            did_op = true;
        }
        if did_op {
            // Real W25Q flash clears WEL when the program/erase completes
            self.wel = false;
            self.status1 &= !0x02;
        }
    }

    fn start_command(&mut self, cmd: Command) {
        self.dummy_pending = (1 + cmd.arg_count().unwrap_or(0)) as u8;
        self.cmd = Some((cmd, vec![]));
        if cmd.arg_count() == Some(0) {
            if let Some(reply) = self.apply_command(cmd, &[]) {
                self.reply = Some(reply);
            }
            self.cmd = None;
        }
    }

    fn apply_command(&mut self, cmd: Command, args: &[u8]) -> Option<Reply> {
        use Command::*;
        match cmd {
            WriteEnable => {
                self.wel = true;
                self.status1 |= 0x02;
                None
            }
            WriteDisable => {
                self.wel = false;
                self.status1 &= !0x02;
                None
            }
            WriteStatus1 => {
                // bit1 (WEL) only — WIP bit0 is managed by the model
                if let Some(&v) = args.first() {
                    self.wel = v & 0x02 != 0;
                    self.status1 = v;
                }
                None
            }
            WriteStatus2 => None,
            ChipErase => {
                if self.wel {
                    self.pending_erase = Some((0, self.config.size));
                }
                None
            }
            SectorErase4k | BlockErase32k | BlockErase64k => {
                if self.wel && args.len() >= 3 {
                    let addr = Self::addr24(args);
                    let len = match cmd {
                        SectorErase4k => 0x1000,
                        BlockErase32k => 0x8000,
                        _ => 0x10000,
                    };
                    self.pending_erase = Some((addr & !(len - 1), len));
                }
                None
            }
            PageProgram | QuadPageProgram => {
                if self.wel && args.len() >= 4 {
                    let addr = Self::addr24(args);
                    let mut data = args[3..].to_vec();
                    data.truncate(256);
                    self.pending_program = Some(Program { addr, data });
                }
                None
            }
            ReadId => {
                let jedec = self.config.jedec_id;
                let mut reply = VecDeque::new();
                reply.push_back((jedec >> 16) as u8);
                reply.push_back((jedec >> 8) as u8);
                reply.push_back(jedec as u8);
                Some(Reply::Data(reply))
            }
            ReadDeviceId => {
                let mut reply = VecDeque::new();
                reply.push_back(0xAA);
                reply.push_back(0xBB);
                Some(Reply::Data(reply))
            }
            ReadStatus1 => {
                let mut reply = VecDeque::new();
                reply.push_back(self.status1);
                Some(Reply::Data(reply))
            }
            ReadStatus2 => {
                let mut reply = VecDeque::new();
                reply.push_back(0x00);
                Some(Reply::Data(reply))
            }
            ReadData | FastRead => {
                let addr = Self::addr24(args);
                Some(Reply::FileContent(addr % self.config.size))
            }
            DeepPowerDown | ReleasePowerDown => None,
        }
    }
}

impl ExtDevice<(), u8> for SpiFlash {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} spi-flash", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 {
        if self.dummy_pending > 0 {
            self.dummy_pending -= 1;
            return 0;
        }
        match self.reply.as_mut() {
            Some(Reply::Data(d)) => d.pop_front().unwrap_or_default(),
            Some(Reply::FileContent(addr)) => {
                let c = self.config.content[*addr];
                *addr = (*addr + 1) % self.config.size;
                c
            }
            None => 0,
        }
    }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        if let Some((cmd, args)) = self.cmd.take() {
            match cmd.arg_count() {
                // PageProgram/QuadPageProgram: fixed address prefix, then
                // data bytes buffered until CS deasserts.
                Some(n) if cmd == Command::PageProgram || cmd == Command::QuadPageProgram => {
                    if args.len() < n {
                        let mut args = args;
                        args.push(v);
                        self.cmd = Some((cmd, args));
                    } else {
                        let mut args = args;
                        args.push(v);
                        let cap = n + 256;
                        if args.len() > cap {
                            args.truncate(cap);
                        }
                        self.cmd = Some((cmd, args.clone()));
                        self.apply_command(cmd, &args);
                    }
                }
                Some(n) => {
                    let mut args = args;
                    args.push(v);
                    if args.len() >= n {
                        if let Some(reply) = self.apply_command(cmd, &args) {
                            self.reply = Some(reply);
                        }
                    } else {
                        self.cmd = Some((cmd, args));
                    }
                }
                None => unreachable!(),
            }
        } else if let Ok(cmd) = Command::try_from(v) {
            self.start_command(cmd);
        }
    }

    fn cs_changed(&mut self, _sys: &System, asserted: bool) {
        self.cs_state = asserted;
        if !asserted {
            // CS rising: terminate any buffered transaction
            self.commit_pending();
            self.cmd = None;
            self.reply = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jedec_and_status_flow() {
        let mut f = SpiFlash::new(SpiFlashConfig {
            peripheral: "SPI3".into(), jedec_id: 0xEF4015,
            content: vec![0xFF; 4096], size: 4096, cs: Some("PB12".into()),
        });
        f.connect_peripheral("SPI3");
        let sys = crate::system::test_dummy_system();
        // command + 3 dummy byte transfers, popping the reply each write as
        // the SPI driver does (write -> read pair). First pop is the dummy
        // byte for the command byte period.
        let mut pops = Vec::new();
        for b in [0x9Fu8, 0x00, 0x00, 0x00] {
            f.write(&sys, (), b);
            pops.push(f.read(&sys, ()));
        }
        assert_eq!(pops, vec![0x00, 0xEF, 0x40, 0x15], "jedec readback");
        // status
        let mut pops = Vec::new();
        for b in [0x05u8, 0x00] {
            f.write(&sys, (), b);
            pops.push(f.read(&sys, ()));
        }
        assert_eq!(pops, vec![0x00, 0x00], "status no WEL");
        // write enable
        f.write(&sys, (), 0x06);
        let mut pops = Vec::new();
        for b in [0x05u8, 0x00] {
            f.write(&sys, (), b);
            pops.push(f.read(&sys, ()));
        }
        assert_eq!(pops, vec![0x00, 0x02], "status WEL");
    }

    #[test]
    fn page_program_commit_on_cs_deassert() {
        let mut f = SpiFlash::new(SpiFlashConfig {
            peripheral: "SPI3".into(), jedec_id: 0xEF4015,
            content: vec![0xFF; 4096], size: 4096, cs: Some("PB12".into()),
        });
        f.connect_peripheral("SPI3");
        let sys = crate::system::test_dummy_system();
        f.write(&sys, (), 0x06); // write enable
        f.cs_changed(&sys, true);
        f.write(&sys, (), 0x02); // page program
        f.write(&sys, (), 0x00); f.write(&sys, (), 0x00); f.write(&sys, (), 0x10);
        f.write(&sys, (), b'A'); f.write(&sys, (), b'B'); f.write(&sys, (), b'C');
        f.cs_changed(&sys, false); // commit
        assert_eq!(&f.config.content[0x10..0x13], b"ABC", "programmed");
        assert_eq!(f.config.content[0x0F], 0xFF, "untouched");
        // read back: 0x03 cmd + 3 addr bytes + 3 data byte transfers, each
        // write pops one MISO byte (4 dummies first)
        f.cs_changed(&sys, true);
        let mut pops = Vec::new();
        for b in [0x03u8, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00] {
            f.write(&sys, (), b);
            pops.push(f.read(&sys, ()));
        }
        assert_eq!(pops, vec![0x00, 0x00, 0x00, 0x00, b'A', b'B', b'C'], "readback");
        f.cs_changed(&sys, false);
    }

    #[test]
    fn sector_erase_requires_wel() {
        let mut f = SpiFlash::new(SpiFlashConfig {
            peripheral: "SPI3".into(), jedec_id: 0xEF4015,
            content: vec![0x11; 4096], size: 4096, cs: Some("PB12".into()),
        });
        f.connect_peripheral("SPI3");
        let sys = crate::system::test_dummy_system();
        // without WEL: no erase
        f.write(&sys, (), 0x20); f.write(&sys, (), 0x00); f.write(&sys, (), 0x00); f.write(&sys, (), 0x00);
        f.cs_changed(&sys, false);
        assert_eq!(f.config.content[0], 0x11, "no erase without WEL");
        // with WEL: 4k erase
        f.write(&sys, (), 0x06);
        f.cs_changed(&sys, true);
        f.write(&sys, (), 0x20); f.write(&sys, (), 0x00); f.write(&sys, (), 0x00); f.write(&sys, (), 0x00);
        f.cs_changed(&sys, false);
        assert_eq!(f.config.content[0], 0xFF, "erased");
    }
}
