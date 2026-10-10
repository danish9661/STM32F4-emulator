use std::collections::VecDeque;
use crate::system::System;
use super::ExtDevice;

pub struct SdCardConfig {
    pub peripheral: String,
    pub blocks: usize, // 512-byte blocks
    pub cs: Option<String>,
}

/// SD card in SPI mode, answered synchronously per byte (like SpiFlash).
///
/// WHY THIS EXISTS (not a JS tap): the JS tap path (`spi_tap` +
/// `parseSpi` + `spi_push_miso`) round-trips through the event queue, which
/// the driver only drains BETWEEN `cpu.step()` batches — while the Rust
/// engine pops the MISO queue synchronously per DR write DURING the step
/// (`spi.rs` master path: `d.write()` then `d.read()` per byte). A whole
/// CMD17 data phase (6 cmd bytes + R1 poll + token + 512 data + CRC)
/// fits inside one coarse execute batch, so JS-computed replies always
/// land one transfer late: init converges only because CMD0/CMD8/ACMD41
/// are retried across steps, single-block reads never retry. Step quanta
/// cannot fix it structurally (fine quanta break init timing elsewhere).
/// A model-side device sees every byte in order and answers in the same
/// transfer — the SpiFlash contract. For JS-driven custom protocols the
/// `spi_clear_miso` + `spi_push_miso` prefill contract covers
/// across-step transactions (see the facade `spi.clearMiso` note).
///
/// SDHC block addressing only (CCS=1 in the ACMD41/CMD58 OCR): CMD17/24
/// ARG is a block index. Out-of-range blocks answer R1 address-error
/// (0x20) with no token, like silicon. Ncr=1: every command response is
/// preceded by one idle byte (silicon's 1-8 Ncr window, deterministic) —
/// discard-first drivers (SdFat) drop it, poll-until-non-FF drivers skip
/// it; Ncr=0 starved SdFat because its discard ate the R1 itself.
pub struct SdCard {
    pub config: SdCardConfig,
    name: String,
    content: Vec<u8>, // blocks*512, erased 0xFF
    cmd: Option<(u8, Vec<u8>)>, // (cmd index, post-token bytes incl. CRC)
    reply: VecDeque<u8>,        // queued MISO bytes
    idle: bool,          // set by CMD0, cleared when ACMD41 reports ready
    app_cmd: bool,       // CMD55 prefix latched for the next command
    acmd41_seen: bool,   // first ACMD41 answers busy (0x01), later ones ready
    /// CMD24 host-to-card collection: token + 512 data + 2 CRC.
    wr: Option<WriteStage>,
    /// CMD25 multi-block write: after each packet commits, a fresh stage
    /// re-arms at block+1 (silicon auto-increments) until the 0xFD
    /// STOP_TRAN token ends the stream. SdFat DEDICATED_SPI routes EVERY
    /// write (even single-sector) through CMD25, so without this the
    /// Arduino path fails at writeStart (R1 illegal 0x04) while raw
    /// CMD24 tests stay green — exactly the observed split.
    multi_write: bool,
    /// CMD18 streaming: next block index to queue when the reply drains.
    stream_next: Option<u32>,
    cs_state: bool,
    /// CMD32/33 erase window (block indices, SDHC). CMD38 accepted as a
    /// no-op clearing the window (content untouched) — SdFat probes the
    /// erase path during init on some cards; answering illegal (0x04)
    /// makes it retry until its timeout looks like a hang.
    erase_start: Option<u32>,
    erase_end: Option<u32>,
}

struct WriteStage {
    block: u32,
    buf: Vec<u8>, // data bytes collected after the 0xFE token (incl. CRC tail)
    got_token: bool,
}

impl SdCard {
    pub fn new(config: SdCardConfig) -> Self {
        let blocks = config.blocks.max(1);
        SdCard {
            content: vec![0xFF; blocks * 512],
            config,
            name: String::new(),
            cmd: None,
            reply: VecDeque::new(),
            idle: true, // powers up idle (needs CMD0 like silicon)
            app_cmd: false,
            acmd41_seen: false,
            erase_start: None,
            erase_end: None,
            wr: None,
            multi_write: false,
            stream_next: None,
            cs_state: true,
        }
    }

    pub fn block_count(&self) -> usize {
        self.content.len() / 512
    }

    /// Overwrite card content with a filesystem image (FS-level tests).
    /// Longer images are truncated to the card size; shorter ones leave
    /// the tail erased (0xFF). The block count never changes.
    pub fn load_image(&mut self, data: &[u8]) {
        let n = data.len().min(self.content.len());
        self.content[..n].copy_from_slice(&data[..n]);
    }

    fn push_r1(&mut self, v: u8) {
        self.reply.push_back(v);
    }

    /// CSD v2.0 (SDHC): C_SIZE = blocks/1024 saturating (capacity
    /// (C_SIZE+1)*512KiB). Tests pin the capacity math, not the bytes.
    fn csd(&self) -> [u8; 16] {
        let csize = ((self.block_count() / 1024).saturating_sub(1)) as u32 & 0x3F_FFFF;
        [
            0x40, 0x0E, 0x00, 0x32, 0x5B, 0x59, 0x09,
            ((csize >> 16) & 0x3F) as u8,
            ((csize >> 8) & 0xFF) as u8,
            (csize & 0xFF) as u8,
            0x7F, 0x80, 0x0A, 0x40, 0x00, 0x00,
        ]
    }

    fn cid() -> [u8; 16] {
        // MID 0x03 (SanDisk), OID "SD", PNM "SD04G", rest canned.
        [0x03, b'S', b'D', b'S', b'D', b'0', b'4', b'G',
         0x10, 0x12, 0x34, 0x56, 0x78, 0x01, 0x1A, 0x00]
    }

    fn queue_data_block(&mut self, block: u32) {
        // Immediate 0xFE token (Ncr=0): every poll loop waits for the
        // token, so zero delay is the friendliest deterministic choice.
        self.reply.push_back(0xFE);
        let o = block as usize * 512;
        for i in 0..512 {
            self.reply.push_back(*self.content.get(o + i).unwrap_or(&0xFF));
        }
        self.reply.push_back(0xFF); // CRC16 (dummy — data integrity is exact)
        self.reply.push_back(0xFF);
    }

    fn decode(&mut self, idx: u8, args: &[u8]) {
        // Ncr=2: every command response is preceded by TWO idle bytes.
        // Silicon answers 1-8 bytes after the command, and TWO consumers
        // each eat one byte before R1: (1) the CRC byte's own paired read
        // (decode runs during that write, so its read pops the queue
        // head), and (2) the driver's discard-first read (SdFat drops one
        // fill byte after every command). Ncr=1 starved SdFat: its discard
        // ate the R1 itself and init could never succeed. Poll-until-
        // non-FF drivers skip both pads harmlessly.
        self.reply.push_back(0xFF);
        self.reply.push_back(0xFF);
        let arg = ((args[0] as u32) << 24) | ((args[1] as u32) << 16)
            | ((args[2] as u32) << 8) | args[3] as u32;
        let in_range = (arg as usize) < self.block_count();
        match idx {
            // CMD0 GO_IDLE_STATE: reset to idle (R1 = 0x01).
            0 => {
                self.idle = true;
                self.app_cmd = false;
                self.acmd41_seen = false;
                self.wr = None;
                self.multi_write = false;
                self.stream_next = None;
                self.push_r1(0x01);
            }
            // CMD8 SEND_IF_COND: R1 idle + R7 echo of the voltage pattern.
            8 => self.reply.extend([0x01, 0x00, 0x00, 0x01, 0xAA]),
            // CMD55 APP_CMD: latch prefix for the next command.
            55 => {
                self.push_r1(if self.idle { 0x01 } else { 0x00 });
                self.app_cmd = true;
            }
            // ACMD41 SD_SEND_OP_COND (requires CMD55 prefix): busy once,
            // then ready. Without the prefix: illegal-command R1.
            41 if self.app_cmd => {
                self.app_cmd = false;
                if !self.acmd41_seen {
                    self.acmd41_seen = true;
                    self.push_r1(0x01);
                } else {
                    self.idle = false;
                    self.push_r1(0x00);
                }
            }
            // CMD58 READ_OCR: R1 + OCR with CCS=1 (SDHC block addressing).
            58 => self.reply.extend([if self.idle { 0x01 } else { 0x00 }, 0xC0, 0xFF, 0x80, 0x00]),
            // CMD16 SET_BLOCKLEN: fixed 512 in this model.
            16 => self.push_r1(if arg == 512 { 0x00 } else { 0x40 }),
            // CMD59 CRC_ON_OFF: CRC is never checked in this model, so
            // both states are accepted (R1). SdFat disables CRC via
            // CMD59(0) during init; answering illegal (0x04) makes it
            // retry until its timeout looks like a hang.
            59 => self.push_r1(if self.idle { 0x01 } else { 0x00 }),
            // CMD32/33 ERASE_WR_BLK_START/END + CMD38 ERASE: accepted as
            // a no-op (window latched, then cleared; content untouched).
            32 => {
                self.erase_start = Some(arg);
                self.push_r1(if self.idle { 0x01 } else { 0x00 });
            }
            33 => {
                self.erase_end = Some(arg);
                self.push_r1(if self.idle { 0x01 } else { 0x00 });
            }
            38 => {
                self.erase_start = None;
                self.erase_end = None;
                self.push_r1(if self.idle { 0x01 } else { 0x00 });
            }
            // CMD13 SEND_STATUS: R2 (all clear).
            13 => self.reply.extend([0x00, 0x00]),
            // CMD9 SEND_CSD / CMD10 SEND_CID: R1 + token + 16 bytes + CRC.
            9 => {
                self.push_r1(0x00);
                self.reply.push_back(0xFE);
                self.reply.extend(self.csd());
                self.reply.extend([0xFF, 0xFF]);
            }
            10 => {
                self.push_r1(0x00);
                self.reply.push_back(0xFE);
                self.reply.extend(Self::cid());
                self.reply.extend([0xFF, 0xFF]);
            }
            // CMD17 READ_SINGLE_BLOCK: R1 + token + 512 + CRC.
            17 => {
                if !in_range {
                    self.push_r1(0x20); // address error, no token
                } else {
                    self.push_r1(0x00);
                    self.queue_data_block(arg);
                }
            }
            // CMD18 READ_MULTIPLE_BLOCK: stream blocks until CMD12.
            18 => {
                if !in_range {
                    self.push_r1(0x20);
                } else {
                    self.push_r1(0x00);
                    self.queue_data_block(arg);
                    self.stream_next = Some(arg + 1);
                }
            }
            // CMD12 STOP_TRANSMISSION: ends a CMD18 stream (and defensively
            // any write stream — the write path normally ends via 0xFD).
            12 => {
                self.stream_next = None;
                self.wr = None;
                self.multi_write = false;
                self.push_r1(0x00);
            }
            // CMD24 WRITE_BLOCK: R1, then collect token + 512 + CRC.
            24 => {
                if !in_range {
                    self.push_r1(0x20);
                } else {
                    self.push_r1(0x00);
                    self.multi_write = false;
                    self.wr = Some(WriteStage { block: arg, buf: Vec::new(), got_token: false });
                }
            }
            // CMD25 WRITE_MULTIPLE_BLOCK: R1, then per-packet [token +
            // 512 + CRC] until the 0xFD STOP_TRAN token (no CMD12 on the
            // write path — SdFat ends with the token, then CS high).
            25 => {
                if !in_range {
                    self.push_r1(0x20);
                } else {
                    self.push_r1(0x00);
                    self.multi_write = true;
                    self.wr = Some(WriteStage { block: arg, buf: Vec::new(), got_token: false });
                }
            }
            // Bare ACMD41 (no CMD55 prefix): illegal-command, no state change.
            41 => self.push_r1(0x04),
            // ACMD23 SET_WR_BLK_ERASE_COUNT (requires CMD55 prefix):
            // pre-erase count preceding a CMD25 stream. writeStart sends
            // CMD55 + CMD23 before CMD25; answering illegal (0x04) here
            // fails the open-for-write path while single-block CMD24
            // writes stay green — the same split shape as CMD25.
            23 if self.app_cmd => {
                self.app_cmd = false;
                self.push_r1(if self.idle { 0x01 } else { 0x00 });
            }
            // Bare CMD23 (no CMD55 prefix): illegal-command, like silicon.
            23 => self.push_r1(0x04),
            _ => self.push_r1(0x04), // illegal command
        }
    }
}

impl ExtDevice<(), u8> for SdCard {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} sd-card", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 {
        // CMD18 streaming: refill the next block as the reply drains.
        if self.reply.is_empty() {
            if let Some(b) = self.stream_next {
                if (b as usize) < self.block_count() {
                    self.queue_data_block(b);
                    self.stream_next = Some(b + 1);
                } else {
                    self.stream_next = None; // ran off the card: idle-high
                }
            }
        }
        self.reply.pop_front().unwrap_or(0xFF)
    }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        // Host-to-card data collection: skip leading 0xFF, take a start
        // token (0xFE single-block, 0xFC multi-block — the latter only
        // inside a CMD25 stream), then 512 data + 2 CRC bytes. 0xFD ends
        // a CMD25 stream (STOP_TRAN — no CMD12 on the write path).
        if let Some(wr) = self.wr.as_mut() {
            if !wr.got_token {
                if v == 0xFE || (v == 0xFC && self.multi_write) {
                    wr.got_token = true;
                } else if v == 0xFD && self.multi_write {
                    self.wr = None;
                    self.multi_write = false;
                    for _ in 0..4 {
                        self.reply.push_back(0x00); // post-STOP busy
                    }
                }
                return; // token wait reads idle-high via reply-empty path
            }
            wr.buf.push(v);
            if wr.buf.len() == 514 {
                let block = wr.block as usize * 512;
                for (i, b) in wr.buf[..512].iter().enumerate() {
                    if let Some(c) = self.content.get_mut(block + i) {
                        *c = *b;
                    }
                }
                let accepted = wr.block as usize * 512 < self.content.len();
                let next = wr.block + 1;
                let multi = self.multi_write;
                self.wr = if multi {
                    // Silicon auto-increments: re-arm for the next packet.
                    Some(WriteStage { block: next, buf: Vec::new(), got_token: false })
                } else {
                    None
                };
                // Data-response token + busy clocks, then idle-high.
                // One leading 0xFF: the CRC-byte-paired read must not eat
                // the response (silicon answers on the NEXT clock).
                self.reply.push_back(0xFF);
                self.reply.push_back(if accepted { 0xE5 } else { 0x0D });
                for _ in 0..4 {
                    self.reply.push_back(0x00); // programming busy
                }
            }
            return;
        }
        if let Some((idx, mut args)) = self.cmd.take() {
            // Post-token frame: 4 ARG bytes + CRC (5 total). The CRC byte
            // is consumed here — NOT re-decoded as a new command token
            // (a CRC value with 01xxxxxx bits would otherwise phantom-start
            // a command and desync the stream).
            args.push(v);
            if args.len() >= 5 {
                let full = args.clone();
                self.cmd = None;
                self.decode(idx, &full[..4]);
            } else {
                self.cmd = Some((idx, args));
            }
            return;
        }
        // Command token: 0b01xxxxxx. 0xFF fillers (and stray bytes)
        // are ignored — the bus idles high between transfers.
        if v & 0xC0 == 0x40 {
            let idx = v & 0x3F;
            self.cmd = Some((idx, Vec::new()));
        }
    }

    fn cs_changed(&mut self, _sys: &System, asserted: bool) {
        self.cs_state = asserted;
        if !asserted {
            // CS rising: drop framing, replies, and partial writes —
            // commits already landed at packet-complete (like flash).
            self.cmd = None;
            self.reply.clear();
            self.wr = None;
            self.multi_write = false;
            self.stream_next = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> (SdCard, RcTestSys) {
        let mut c = SdCard::new(SdCardConfig {
            peripheral: "SPI1".into(),
            blocks: 4,
            cs: None,
        });
        c.connect_peripheral("SPI1");
        let sys = crate::system::test_dummy_system();
        (c, sys)
    }
    type RcTestSys = std::rc::Rc<crate::system::System>;

    /// One full-duplex byte: write clocks MOSI in, read returns MISO —
    /// exactly the pair the SPI master path performs per DR write.
    fn xfer(c: &mut SdCard, sys: &RcTestSys, b: u8) -> u8 {
        c.write(sys, (), b);
        c.read(sys, ())
    }

    /// Clock a 6-byte command frame exactly like SdFat's cardCommand:
    /// 6 paired reads (ignored, like spiSend's), one discard read, then
    /// poll up to 10 reads for the first non-0xFF byte. Returns that byte
    /// (R1). This shape is the regression pin for the discard-eats-R1
    /// class: with Ncr<2 the discard would eat R1 and this helper could
    /// never observe anything but 0xFF.
    fn cmd(c: &mut SdCard, sys: &RcTestSys, idx: u8, arg: u32) -> u8 {
        for b in [0x40 | idx,
            (arg >> 24) as u8, (arg >> 16) as u8, (arg >> 8) as u8, arg as u8,
            0xFF] {
            xfer(c, sys, b);
        }
        xfer(c, sys, 0xFF); // discard first fill (SdFat drops one byte)
        for _ in 0..10 {
            let r = xfer(c, sys, 0xFF);
            if r != 0xFF {
                return r;
            }
        }
        0xFF
    }

    #[test]
    fn init_sequence_goes_ready() {
        let (mut c, sys) = card();
        // Raw wire order pin: 6 frame clocks idle-high, Ncr pads, then R1.
        let mut raw = vec![];
        for b in [0x40u8, 0, 0, 0, 0, 0x95] {
            raw.push(xfer(&mut c, &sys, b));
        }
        assert_eq!(raw, vec![0xFF; 6], "frame clocks idle-high");
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xFF, "Ncr pad (discard position)");
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0x01, "CMD0 R1 follows Ncr");
        // CMD8 -> R1 + R7 echo.
        assert_eq!(cmd(&mut c, &sys, 8, 0x1AA), 0x01, "CMD8 R1");
        let mut r7 = vec![];
        for _ in 0..4 {
            r7.push(xfer(&mut c, &sys, 0xFF));
        }
        assert_eq!(r7, vec![0x00, 0x00, 0x01, 0xAA], "CMD8 R7 echo");
        // CMD55 + ACMD41 -> busy once, then ready.
        assert_eq!(cmd(&mut c, &sys, 55, 0), 0x01, "CMD55 R1 idle");
        assert_eq!(cmd(&mut c, &sys, 41, 0), 0x01, "ACMD41 busy");
        assert_eq!(cmd(&mut c, &sys, 55, 0), 0x01, "CMD55 R1");
        assert_eq!(cmd(&mut c, &sys, 41, 0), 0x00, "ACMD41 ready");
        // CMD58 OCR carries CCS (SDHC block addressing).
        assert_eq!(cmd(&mut c, &sys, 58, 0), 0x00, "CMD58 R1");
        let mut ocr = vec![];
        for _ in 0..4 {
            ocr.push(xfer(&mut c, &sys, 0xFF));
        }
        assert_eq!(ocr[0] & 0x40, 0x40, "OCR CCS set");
    }

    #[test]
    fn cmd17_reads_exact_block() {
        let (mut c, sys) = card();
        // Ready the card via the SdFat-shaped helper.
        assert_eq!(cmd(&mut c, &sys, 0, 0), 0x01, "CMD0 idle");
        assert_eq!(cmd(&mut c, &sys, 55, 0), 0x01, "CMD55");
        assert_eq!(cmd(&mut c, &sys, 41, 0), 0x01, "ACMD41 busy");
        assert_eq!(cmd(&mut c, &sys, 55, 0), 0x01, "CMD55");
        assert_eq!(cmd(&mut c, &sys, 41, 0), 0x00, "ACMD41 ready");
        for i in 0..512 {
            c.content[2 * 512 + i] = (i ^ 0xA5) as u8;
        }
        // CMD17 block 2: R1, token, 512 exact bytes, 2 CRC.
        assert_eq!(cmd(&mut c, &sys, 17, 2), 0x00, "CMD17 R1");
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xFE, "data token");
        for i in 0..512 {
            assert_eq!(xfer(&mut c, &sys, 0xFF), (i ^ 0xA5) as u8, "byte {i}");
        }
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xFF, "CRC0");
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xFF, "CRC1");
    }

    #[test]
    fn cmd24_write_roundtrips_through_cmd17() {
        let (mut c, sys) = card();
        assert_eq!(cmd(&mut c, &sys, 0, 0), 0x01, "CMD0 idle");
        // CMD24 block 1 with a ramp pattern.
        assert_eq!(cmd(&mut c, &sys, 24, 1), 0x00, "CMD24 R1");
        xfer(&mut c, &sys, 0xFF); // Nwr spacing
        xfer(&mut c, &sys, 0xFE); // data token
        for i in 0..512 {
            xfer(&mut c, &sys, i as u8);
        }
        xfer(&mut c, &sys, 0xFF); // CRC pair lands the commit on this read
        xfer(&mut c, &sys, 0xFF);
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xE5, "data accepted");
        // Read it back.
        assert_eq!(cmd(&mut c, &sys, 17, 1), 0x00, "CMD17 R1");
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xFE, "token");
        for i in 0..512 {
            assert_eq!(xfer(&mut c, &sys, 0xFF), i as u8, "byte {i}");
        }
    }

    #[test]
    fn erased_reads_ff_and_oob_errors() {
        let (mut c, sys) = card();
        assert_eq!(cmd(&mut c, &sys, 0, 0), 0x01, "CMD0 idle");
        // Fresh card: erased.
        assert_eq!(cmd(&mut c, &sys, 17, 0), 0x00, "R1");
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xFE, "token");
        for _ in 0..512 {
            assert_eq!(xfer(&mut c, &sys, 0xFF), 0xFF, "erased");
        }
        // Out of range: address error, no token.
        assert_eq!(cmd(&mut c, &sys, 17, 99), 0x20, "address error");
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xFF, "no token after error");
        // Bare ACMD41 (no CMD55): illegal, stays idle.
        assert_eq!(cmd(&mut c, &sys, 41, 0), 0x04, "illegal without prefix");
    }

    #[test]
    fn crc_and_erase_cmds_answer_r1() {
        // SdFat init probes CMD59 (CRC off) and the CMD32/33/38 erase
        // path; illegal-command answers here made it retry into its
        // timeout (the SdFat hang). Seeded-image content is untouched.
        let (mut c, sys) = card();
        assert_eq!(cmd(&mut c, &sys, 0, 0), 0x01, "CMD0 idle");
        assert_eq!(cmd(&mut c, &sys, 59, 0), 0x01, "CMD59 accepted while idle");
        assert_eq!(cmd(&mut c, &sys, 32, 0), 0x01, "CMD32 accepted while idle");
        assert_eq!(cmd(&mut c, &sys, 33, 1), 0x01, "CMD33 accepted while idle");
        assert_eq!(cmd(&mut c, &sys, 38, 0), 0x01, "CMD38 accepted while idle");
        // Ready state answers 0x00.
        cmd(&mut c, &sys, 55, 0); cmd(&mut c, &sys, 41, 0);
        cmd(&mut c, &sys, 55, 0);
        assert_eq!(cmd(&mut c, &sys, 41, 0), 0x00, "ACMD41 ready");
        assert_eq!(cmd(&mut c, &sys, 59, 0), 0x00, "CMD59 accepted when ready");
        assert_eq!(cmd(&mut c, &sys, 32, 0), 0x00, "CMD32 accepted when ready");
        assert_eq!(cmd(&mut c, &sys, 33, 1), 0x00, "CMD33 accepted when ready");
        assert_eq!(cmd(&mut c, &sys, 38, 0), 0x00, "CMD38 accepted when ready");
        // Erase window is a no-op: content untouched, still erased.
        assert_eq!(c.content.iter().all(|&x| x == 0xFF), true, "erase is a no-op");
    }

    #[test]
    fn load_image_seeds_content() {
        let (mut c, sys) = card();
        let mut img = vec![0xFF; 4 * 512];
        img[510] = 0x55; img[511] = 0xAA;
        img[512] = 0x42;
        c.load_image(&img);
        assert_eq!(cmd(&mut c, &sys, 17, 0), 0x00, "R1");
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xFE, "token");
        let mut blk = vec![];
        for _ in 0..512 { blk.push(xfer(&mut c, &sys, 0xFF)); }
        assert_eq!(blk[510], 0x55, "seeded signature byte");
        assert_eq!(blk[511], 0xAA, "seeded signature byte");
        // Longer images truncate to the card size (no panic, no growth).
        c.load_image(&vec![0x11; 99 * 512]);
        assert_eq!(c.content.len(), 4 * 512, "size unchanged after oversize load");
    }

    #[test]
    fn acmd23_gated_on_cmd55_prefix() {
        // writeStart sends CMD55 + CMD23 (pre-erase count) before CMD25.
        let (mut c, sys) = card();
        assert_eq!(cmd(&mut c, &sys, 0, 0), 0x01, "CMD0 idle");
        assert_eq!(cmd(&mut c, &sys, 23, 1), 0x04, "bare CMD23 illegal");
        assert_eq!(cmd(&mut c, &sys, 55, 0), 0x01, "CMD55 R1 idle");
        assert_eq!(cmd(&mut c, &sys, 23, 1), 0x01, "ACMD23 accepted while idle");
        // Ready state answers 0x00.
        cmd(&mut c, &sys, 55, 0); cmd(&mut c, &sys, 41, 0);
        cmd(&mut c, &sys, 55, 0);
        assert_eq!(cmd(&mut c, &sys, 41, 0), 0x00, "ACMD41 ready");
        assert_eq!(cmd(&mut c, &sys, 55, 0), 0x00, "CMD55 R1 ready");
        assert_eq!(cmd(&mut c, &sys, 23, 2), 0x00, "ACMD23 accepted when ready");
    }

    #[test]
    fn cmd25_multi_write_streams_until_stop() {
        // SdFat DEDICATED_SPI routes EVERY write (even single-sector)
        // through CMD25 + STOP_TRAN: without this, writeStart fails R1
        // illegal while raw CMD24 tests stay green.
        let (mut c, sys) = card();
        assert_eq!(cmd(&mut c, &sys, 0, 0), 0x01, "CMD0 idle");
        assert_eq!(cmd(&mut c, &sys, 25, 1), 0x00, "CMD25 R1");
        // Packet 1 -> block 1.
        xfer(&mut c, &sys, 0xFF);
        xfer(&mut c, &sys, 0xFC);
        for i in 0..512 {
            xfer(&mut c, &sys, i as u8);
        }
        xfer(&mut c, &sys, 0xFF);
        xfer(&mut c, &sys, 0xFF);
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xE5, "packet 1 accepted");
        // Packet 2 auto-targets block 2 (no new command).
        for _ in 0..4 {
            xfer(&mut c, &sys, 0xFF); // busy drain
        }
        xfer(&mut c, &sys, 0xFC);
        for i in 0..512 {
            xfer(&mut c, &sys, (i ^ 0xFF) as u8);
        }
        xfer(&mut c, &sys, 0xFF);
        xfer(&mut c, &sys, 0xFF);
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xE5, "packet 2 accepted");
        // STOP_TRAN ends the stream (no CMD12 on the write path).
        for _ in 0..4 {
            xfer(&mut c, &sys, 0xFF);
        }
        xfer(&mut c, &sys, 0xFD);
        // Both blocks read back exact.
        assert_eq!(cmd(&mut c, &sys, 17, 1), 0x00, "CMD17 R1");
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xFE, "token");
        for i in 0..512 {
            assert_eq!(xfer(&mut c, &sys, 0xFF), i as u8, "block1 byte {i}");
        }
        assert_eq!(cmd(&mut c, &sys, 17, 2), 0x00, "CMD17 R1");
        assert_eq!(xfer(&mut c, &sys, 0xFF), 0xFE, "token");
        for i in 0..512 {
            assert_eq!(xfer(&mut c, &sys, 0xFF), (i ^ 0xFF) as u8, "block2 byte {i}");
        }
    }

    #[test]
    fn cs_deassert_drops_partial_state() {
        let (mut c, sys) = card();
        cmd(&mut c, &sys, 0, 0);
        xfer(&mut c, &sys, 0xFF);
        // Start CMD17 but abort mid-frame: no reply must leak.
        c.write(&sys, (), 0x51);
        c.write(&sys, (), 0x00);
        c.cs_changed(&sys, false);
        assert_eq!(c.read(&sys, ()), 0xFF, "aborted frame answers idle");
        // Partial CMD24 write aborted: image untouched.
        cmd(&mut c, &sys, 24, 0);
        xfer(&mut c, &sys, 0xFF);
        xfer(&mut c, &sys, 0xFE);
        xfer(&mut c, &sys, 0x11);
        c.cs_changed(&sys, false);
        assert_eq!(c.content[0], 0xFF, "aborted write dropped");
    }

    /// Full path through the modeled SPI peripheral: GPIO CS + SPI3 DR,
    /// exactly like guest firmware drives it. CMD17 must return 512 exact
    /// bytes in the SAME transfer (the JS tap round-trip cannot do this —
    /// this test is the regression pin for the one-transfer-late class).
    #[test]
    fn cmd17_through_spi_registers() {
        use crate::ext_devices::ExtDevices;
        use std::{rc::Rc, cell::RefCell};
        let dev = Rc::new(RefCell::new(SdCard::new(SdCardConfig {
            peripheral: "SPI3".into(),
            blocks: 4,
            cs: Some("PB12".into()),
        })));
        // Seed block 3 with a ramp before the peripheral binds the device.
        for i in 0..512 {
            dev.borrow_mut().content[3 * 512 + i] = (i ^ 0x3C) as u8;
        }
        let mut ext = ExtDevices::default();
        ext.spi_sds.push(dev);
        let sys = crate::system::test_system_with(&ext);
        fn w(sys: &crate::system::System, addr: u32, v: u32) {
            sys.p.write(sys, addr, 4, v);
        }
        fn r(sys: &crate::system::System, addr: u32) -> u32 {
            sys.p.read(sys, addr, 4)
        }
        // GPIOB: PB12 output (CS), PB13-15 AF5 (SCK/MISO/MOSI).
        w(&sys, 0x40020400, (1u32 << 24) | (2 << 26) | (2 << 28) | (2 << 30));
        w(&sys, 0x40020414, 1 << 12); // CS high (idle)
        w(&sys, 0x40003C00, 0x364); // SPI3 CR1 master
        w(&sys, 0x40003C00, 0x364 | 0x40); // + SPE
        let mut tr = |b: u8| -> u8 {
            w(&sys, 0x40003C0C, b as u32);
            r(&sys, 0x40003C0C) as u8
        };
        // Init: CMD0, CMD55 + ACMD41 to ready — SdFat-shaped
        // (frame + discard + poll), like the `cmd` helper above.
        w(&sys, 0x40020414, 1 << (12 + 16)); // CS low
        let mut cmdr = |idx: u8, arg: u32| -> u8 {
            for b in [0x40 | idx,
                (arg >> 24) as u8, (arg >> 16) as u8, (arg >> 8) as u8, arg as u8, 0xFF] {
                tr(b);
            }
            tr(0xFF); // discard first fill (SdFat drops one byte)
            for _ in 0..10 {
                let v = tr(0xFF);
                if v != 0xFF {
                    return v;
                }
            }
            0xFF
        };
        assert_eq!(cmdr(0, 0), 0x01, "CMD0 R1 idle");
        assert_eq!(cmdr(55, 0), 0x01, "CMD55 R1");
        assert_eq!(cmdr(41, 0), 0x01, "ACMD41 busy");
        assert_eq!(cmdr(55, 0), 0x01, "CMD55 R1");
        assert_eq!(cmdr(41, 0), 0x00, "ACMD41 ready");
        // CMD17 block 3, same CS assertion, straight through.
        assert_eq!(cmdr(17, 3), 0x00, "CMD17 R1");
        assert_eq!(tr(0xFF), 0xFE, "data token");
        for i in 0..512 {
            assert_eq!(tr(0xFF), (i ^ 0x3C) as u8, "block byte {i}");
        }
        w(&sys, 0x40020414, 1 << 12); // CS high
    }
}
