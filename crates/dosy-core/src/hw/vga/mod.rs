//! VGA: registers, DAC, 256 KiB VRAM (4 planes), the `A0000–BFFFF` window,
//! display timing (input status 1) and frame production at vretrace.
//!
//! Graphics modes (mode 13h, planar, latches, write modes 1–3) arrive in
//! 026/027; this module renders text modes.

pub mod font;
mod text;

#[cfg(test)]
mod tests;

use super::{DeviceId, IoCtx, IoDevice, Scheduler};
use crate::bus::Addr;
use crate::memory::Mmio;
use crate::video::{Rgb8, VideoFrame};

/// Bytes per plane.
pub const PLANE_SIZE: usize = 0x1_0000;
/// Font bytes per character slot in plane 2.
pub const FONT_STRIDE: usize = 32;

/// Scheduler event kind: start of vertical retrace.
const EV_VRETRACE: u32 = 0;

/// Dot clocks selected by misc output bits 2–3 (Hz).
const DOT_CLOCKS: [u64; 4] = [25_175_000, 28_322_000, 25_175_000, 28_322_000];

// Mode 3 (80x25 text, 720x400 @ 70 Hz) register values, as left by the BIOS.
const SEQ_MODE3: [u8; 5] = [0x03, 0x00, 0x03, 0x00, 0x02];
#[rustfmt::skip]
const CRTC_MODE3: [u8; 25] = [
    0x5F, 0x4F, 0x50, 0x82, 0x55, 0x81, 0xBF, 0x1F, 0x00, 0x4F, 0x0D, 0x0E,
    0x00, 0x00, 0x00, 0x00, 0x9C, 0x8E, 0x8F, 0x28, 0x1F, 0x96, 0xB9, 0xA3, 0xFF,
];
const GC_MODE3: [u8; 9] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00, 0xFF];
#[rustfmt::skip]
const AC_MODE3: [u8; 21] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07, 0x38, 0x39, 0x3A, 0x3B,
    0x3C, 0x3D, 0x3E, 0x3F, 0x0C, 0x00, 0x0F, 0x08, 0x00,
];
const MISC_MODE3: u8 = 0x67;

pub struct Vga {
    /// Plane `p` occupies `p * PLANE_SIZE ..`.
    pub vram: Vec<u8>,
    pub seq: [u8; 5],
    pub crtc: [u8; 25],
    pub gc: [u8; 9],
    pub ac: [u8; 21],
    pub misc: u8,
    /// 6-bit RGB per entry.
    pub dac: [[u8; 3]; 256],
    pub pel_mask: u8,
    seq_index: u8,
    crtc_index: u8,
    gc_index: u8,
    /// Low 5 bits: register; bit 5: palette address source (PAS).
    ac_index: u8,
    /// Attribute controller flip-flop: `true` = next 3C0 write is data.
    ac_data: bool,
    dac_write: u8,
    dac_read: u8,
    /// Component (0..3) within the current DAC entry.
    dac_comp: u8,
    dac_comp_buf: [u8; 3],
    dac_reading: bool,
    frames: u64,
    frame: Option<VideoFrame>,
}

impl Default for Vga {
    fn default() -> Self {
        Self::new()
    }
}

/// 6-bit DAC value to 8 bits.
fn dac8(v: u8) -> u8 {
    (v << 2) | (v >> 4)
}

impl Vga {
    /// Power-on state as left by the BIOS POST: mode 3, 8x16 font in
    /// plane 2, EGA palette in DAC 0–63.
    pub fn new() -> Self {
        let mut v = Self {
            vram: vec![0; 4 * PLANE_SIZE],
            seq: SEQ_MODE3,
            crtc: CRTC_MODE3,
            gc: GC_MODE3,
            ac: AC_MODE3,
            misc: MISC_MODE3,
            dac: [[0; 3]; 256],
            pel_mask: 0xFF,
            seq_index: 0,
            crtc_index: 0,
            gc_index: 0,
            ac_index: 0x20,
            ac_data: false,
            dac_write: 0,
            dac_read: 0,
            dac_comp: 0,
            dac_comp_buf: [0; 3],
            dac_reading: false,
            frames: 0,
            frame: None,
        };
        for i in 0..64u8 {
            let c = |hi: u8, lo: u8| 0x2A * ((i >> hi) & 1) + 0x15 * ((i >> lo) & 1);
            v.dac[i as usize] = [c(2, 5), c(1, 4), c(0, 3)];
        }
        v.load_font(&font::FONT_8X16, 16, 0);
        v
    }

    /// Copy a `height`-row font into plane 2 character map `bank` (0..8).
    pub fn load_font(&mut self, glyphs: &[u8], height: usize, bank: usize) {
        let base = 2 * PLANE_SIZE + bank_offset(bank);
        for (c, g) in glyphs.chunks(height).take(256).enumerate() {
            let at = base + c * FONT_STRIDE;
            self.vram[at..at + height].copy_from_slice(g);
        }
    }

    /// Schedule the first vretrace; `Hardware` calls this once.
    pub fn start(&self, sched: &mut Scheduler) {
        sched.schedule(self.next_retrace(0), DeviceId::VGA, EV_VRETRACE);
    }

    /// The frame completed at the last vretrace, if not taken yet.
    pub fn take_frame(&mut self) -> Option<VideoFrame> {
        self.frame.take()
    }

    pub fn frame(&self) -> Option<&VideoFrame> {
        self.frame.as_ref()
    }

    // --- Timing ---

    fn dot_clock(&self) -> u64 {
        DOT_CLOCKS[((self.misc >> 2) & 3) as usize]
    }
    fn char_width(&self) -> u64 {
        if self.seq[1] & 1 != 0 { 8 } else { 9 }
    }
    /// 10-bit vertical CRTC value from a low byte and two overflow bits.
    fn crtc_v(&self, low: usize, bit8: u8, bit9: u8) -> u64 {
        let of = self.crtc[7];
        let b8 = ((of >> bit8) & 1) as u64;
        let b9 = ((of >> bit9) & 1) as u64;
        self.crtc[low] as u64 | (b8 << 8) | (b9 << 9)
    }
    fn dots_per_line(&self) -> u64 {
        (self.crtc[0] as u64 + 5) * self.char_width()
    }
    fn total_lines(&self) -> u64 {
        self.crtc_v(6, 0, 5) + 2
    }
    /// Active display lines.
    fn display_lines(&self) -> u64 {
        self.crtc_v(0x12, 1, 6) + 1
    }
    fn retrace_line(&self) -> u64 {
        self.crtc_v(0x10, 2, 7)
    }
    fn frame_dots(&self) -> u64 {
        self.dots_per_line() * self.total_lines()
    }
    /// Dots since power-on at `ns` (no drift; timing changes shift phase).
    fn dots_at(&self, ns: u64) -> u64 {
        (ns as u128 * self.dot_clock() as u128 / 1_000_000_000) as u64
    }
    /// First ns at or after which `dots` have elapsed.
    fn ns_at(&self, dots: u64) -> u64 {
        (dots as u128 * 1_000_000_000).div_ceil(self.dot_clock() as u128) as u64
    }

    /// Next vretrace start strictly after `now`.
    fn next_retrace(&self, now: u64) -> u64 {
        let fd = self.frame_dots();
        let at = self.retrace_line() * self.dots_per_line();
        let d = self.dots_at(now);
        let mut t = d - d % fd + at;
        while self.ns_at(t) <= now {
            t += fd;
        }
        self.ns_at(t)
    }

    /// Input status 1 (3BA/3DA): bit 0 = not in active display, bit 3 =
    /// vertical retrace.
    fn input_status1(&self, now: u64) -> u8 {
        let dpl = self.dots_per_line();
        let pos = self.dots_at(now) % self.frame_dots();
        let (line, dot) = (pos / dpl, pos % dpl);
        let hde = (self.crtc[1] as u64 + 1) * self.char_width();
        let vrs = self.retrace_line();
        // Retrace ends when the line counter's low 4 bits match CRTC 11h.
        let vre_len = ((self.crtc[0x11] as u64 & 0xF).wrapping_sub(vrs & 0xF)) & 0xF;
        let in_retrace = line >= vrs && line < vrs + vre_len.max(1);
        let active = line < self.display_lines() && dot < hde;
        (!active as u8) | (in_retrace as u8) << 3
    }

    // --- Memory window ---

    /// Offset inside the window selected by GC 06h, or `None` outside it.
    fn window(&self, addr: Addr) -> Option<usize> {
        let (base, size) = match (self.gc[6] >> 2) & 3 {
            0 => (0xA_0000, 0x2_0000),
            1 => (0xA_0000, 0x1_0000),
            2 => (0xB_0000, 0x8000),
            _ => (0xB_8000, 0x8000),
        };
        let off = addr.checked_sub(base)?;
        (off < size).then_some(off as usize)
    }

    /// Odd/even addressing (text modes): A0 selects plane 0/1.
    fn odd_even(&self) -> bool {
        self.seq[4] & 4 == 0
    }

    fn mem_read(&self, off: usize) -> u8 {
        if self.odd_even() {
            let plane = (off & 1) | (self.gc[4] as usize & 2);
            self.vram[plane * PLANE_SIZE + (off & !1) % PLANE_SIZE]
        } else {
            let plane = self.gc[4] as usize & 3;
            self.vram[plane * PLANE_SIZE + off % PLANE_SIZE]
        }
    }

    fn mem_write(&mut self, off: usize, val: u8) {
        let mask = self.seq[2] & 0xF;
        if self.odd_even() {
            let plane = off & 1;
            for p in [plane, plane | 2] {
                if mask & (1 << p) != 0 {
                    self.vram[p * PLANE_SIZE + (off & !1) % PLANE_SIZE] = val;
                }
            }
        } else {
            for p in 0..4 {
                if mask & (1 << p) != 0 {
                    self.vram[p * PLANE_SIZE + off % PLANE_SIZE] = val;
                }
            }
        }
    }

    // --- Palette ---

    /// Attribute palette index (0..16) to DAC index, per AC 10h/14h.
    pub fn attr_to_dac(&self, i: u8) -> u8 {
        let p = self.ac[i as usize & 0xF];
        let sel = self.ac[0x14];
        let low = if self.ac[0x10] & 0x80 != 0 {
            (p & 0x0F) | ((sel & 3) << 4)
        } else {
            p & 0x3F
        };
        (low | ((sel & 0xC) << 4)) & self.pel_mask
    }

    fn palette(&self) -> [Rgb8; 256] {
        let mut out = [Rgb8::default(); 256];
        for (o, c) in out.iter_mut().zip(&self.dac) {
            *o = Rgb8 {
                r: dac8(c[0]),
                g: dac8(c[1]),
                b: dac8(c[2]),
            };
        }
        out
    }

    fn render(&self) -> VideoFrame {
        let mut f = text::render(self, self.frames);
        f.palette = self.palette();
        f.seq = self.frames;
        f
    }
}

/// Plane 2 offset of character map `bank` (VGA: 0,16K,32K,48K,8K,24K,40K,56K).
fn bank_offset(bank: usize) -> usize {
    ((bank & 3) * 2 + (bank >> 2)) * 0x2000
}

impl Mmio for Vga {
    fn read8(&mut self, addr: Addr) -> Option<u8> {
        Some(self.window(addr).map_or(0xFF, |off| self.mem_read(off)))
    }
    fn write8(&mut self, addr: Addr, val: u8) -> bool {
        if let Some(off) = self.window(addr) {
            self.mem_write(off, val);
        }
        true
    }
}

impl Vga {
    /// Misc output bit 0 selects the CRTC / input status 1 ports: 3Dx
    /// (colour) when set, 3Bx (mono) when clear. The other range floats.
    fn decodes(&self, port: u16) -> bool {
        let color = self.misc & 1 != 0;
        match port {
            0x3B0..=0x3BF => !color,
            0x3D0..=0x3DF => color,
            _ => true,
        }
    }
}

impl IoDevice for Vga {
    fn read(&mut self, port: u16, ctx: &mut IoCtx) -> u8 {
        if !self.decodes(port) {
            return 0xFF;
        }
        match port {
            0x3B4 | 0x3D4 => self.crtc_index,
            0x3B5 | 0x3D5 => self
                .crtc
                .get(self.crtc_index as usize)
                .copied()
                .unwrap_or(0xFF),
            0x3BA | 0x3DA => {
                self.ac_data = false;
                self.input_status1(ctx.now)
            }
            0x3C0 => self.ac_index,
            0x3C1 => self
                .ac
                .get(self.ac_index as usize & 0x1F)
                .copied()
                .unwrap_or(0),
            0x3C2 => 0x10, // input status 0: switch sense
            0x3C4 => self.seq_index,
            0x3C5 => self
                .seq
                .get(self.seq_index as usize)
                .copied()
                .unwrap_or(0xFF),
            0x3C6 => self.pel_mask,
            0x3C7 => {
                if self.dac_reading {
                    0x03
                } else {
                    0x00
                }
            }
            0x3C8 => self.dac_write,
            0x3C9 => {
                let i = self.dac_read as usize;
                let v = self.dac[i][self.dac_comp as usize];
                self.dac_comp += 1;
                if self.dac_comp == 3 {
                    self.dac_comp = 0;
                    self.dac_read = self.dac_read.wrapping_add(1);
                }
                v
            }
            0x3CA => 0x00, // feature control
            0x3CC => self.misc,
            0x3CE => self.gc_index,
            0x3CF => self.gc.get(self.gc_index as usize).copied().unwrap_or(0xFF),
            _ => 0xFF,
        }
    }

    fn write(&mut self, port: u16, val: u8, _ctx: &mut IoCtx) {
        if !self.decodes(port) {
            return;
        }
        match port {
            0x3B4 | 0x3D4 => self.crtc_index = val,
            0x3B5 | 0x3D5 => {
                let i = self.crtc_index as usize;
                // CRTC 11h bit 7 write-protects registers 0–7 (except 07h bit 4).
                let protected = self.crtc[0x11] & 0x80 != 0 && i < 8;
                if i == 7 && protected {
                    self.crtc[7] = (self.crtc[7] & !0x10) | (val & 0x10);
                } else if !protected {
                    if let Some(r) = self.crtc.get_mut(i) {
                        *r = val;
                    }
                }
            }
            0x3C0 => {
                if self.ac_data {
                    if let Some(r) = self.ac.get_mut(self.ac_index as usize & 0x1F) {
                        *r = val;
                    }
                } else {
                    self.ac_index = val & 0x3F;
                }
                self.ac_data = !self.ac_data;
            }
            0x3C2 => self.misc = val,
            0x3C4 => self.seq_index = val,
            0x3C5 => {
                if let Some(r) = self.seq.get_mut(self.seq_index as usize) {
                    *r = val;
                }
            }
            0x3C6 => self.pel_mask = val,
            0x3C7 => {
                self.dac_read = val;
                self.dac_comp = 0;
                self.dac_reading = true;
            }
            0x3C8 => {
                self.dac_write = val;
                self.dac_comp = 0;
                self.dac_reading = false;
            }
            0x3C9 => {
                self.dac_comp_buf[self.dac_comp as usize] = val & 0x3F;
                self.dac_comp += 1;
                if self.dac_comp == 3 {
                    self.dac[self.dac_write as usize] = self.dac_comp_buf;
                    self.dac_comp = 0;
                    self.dac_write = self.dac_write.wrapping_add(1);
                }
            }
            0x3CE => self.gc_index = val,
            0x3CF => {
                if let Some(r) = self.gc.get_mut(self.gc_index as usize) {
                    *r = val;
                }
            }
            _ => {}
        }
    }

    fn event(&mut self, kind: u32, ctx: &mut IoCtx) {
        if kind == EV_VRETRACE {
            self.frames += 1;
            self.frame = Some(self.render());
            ctx.sched
                .schedule(self.next_retrace(ctx.now), DeviceId::VGA, EV_VRETRACE);
        }
    }
}
