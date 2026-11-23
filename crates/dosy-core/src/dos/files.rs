//! Handle-based file I/O and directory services (INT 21h 0Eh, 19h, 36h,
//! 39h–43h, 45h–47h, 56h, 57h, 5Bh). Handles index the JFT in the current
//! PSP; JFT bytes index the host-side system file table.

use super::path::{read_asciiz, write_asciiz};
use super::{Dos, SDA_DRIVE, psp, sda};
use crate::bus::{Bus, linear};
use crate::cpu::{Cpu, Seg};
use crate::vfs::{Attr, DosDateTime, DosError, DosFile, DosPath, OpenMode, SeekFrom};

/// System file table size (`FILES=`).
pub const SFT_LEN: usize = 40;
/// JFT value of an unused handle.
pub const JFT_FREE: u8 = 0xFF;

/// Character devices. CON reads the keyboard and writes the screen (see
/// `console`); NUL, AUX, PRN and CLOCK$ read EOF and discard writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Con,
    Aux,
    Prn,
    Nul,
    Clock,
}

impl Device {
    /// IOCTL 00h device information word.
    pub fn info(self) -> u16 {
        match self {
            // ISDEV | special (INT 29h) | ISCOT | ISCIN, not EOF.
            Device::Con => 0x80D3,
            Device::Nul => 0x8084,
            Device::Clock => 0x8088,
            Device::Aux | Device::Prn => 0x80C0,
        }
    }
}

/// Device named by the last component of a guest path (any directory, any
/// extension): `CON`, `NUL`, `PRN`, `AUX`, `CLOCK$`, `LPTn`, `COMn`.
pub fn device_name(raw: &[u8]) -> Option<Device> {
    let last = raw
        .rsplit(|&c| c == b'\\' || c == b'/' || c == b':')
        .next()?;
    let base = last.split(|&c| c == b'.').next()?;
    let base = base.trim_ascii_end().to_ascii_uppercase();
    Some(match base.as_slice() {
        b"CON" => Device::Con,
        b"NUL" => Device::Nul,
        b"PRN" | b"LPT1" | b"LPT2" | b"LPT3" => Device::Prn,
        b"AUX" | b"COM1" | b"COM2" | b"COM3" | b"COM4" => Device::Aux,
        b"CLOCK$" => Device::Clock,
        _ => return None,
    })
}

pub enum Open {
    Device(Device),
    File {
        file: Box<dyn DosFile>,
        mode: OpenMode,
        drive: u8,
        path: DosPath,
    },
}

/// One system file table entry; `refs` counts JFT slots pointing at it.
pub struct SftEntry {
    pub refs: u16,
    pub open: Open,
    /// Opened with 3Dh mode bit 7: EXEC does not pass it to children.
    pub no_inherit: bool,
    /// Device in raw (binary) mode, IOCTL 01h bit 5.
    pub raw: bool,
}

/// Initial SFT matching `psp::DEFAULT_JFT`: 0 = AUX, 1 = CON, 2 = PRN.
pub fn default_sft() -> Vec<Option<SftEntry>> {
    let mut sft: Vec<Option<SftEntry>> = (0..SFT_LEN).map(|_| None).collect();
    for (i, (dev, refs)) in [(Device::Aux, 1), (Device::Con, 3), (Device::Prn, 1)]
        .into_iter()
        .enumerate()
    {
        sft[i] = Some(SftEntry {
            refs,
            open: Open::Device(dev),
            no_inherit: false,
            raw: false,
        });
    }
    sft
}

type R<T = ()> = Result<T, DosError>;

impl Dos {
    /// Linear address and size of the current process's JFT.
    fn jft<B: Bus>(&self, bus: &mut B) -> (u32, u16) {
        let p = self.current_psp(bus);
        let off = bus.read16(linear(p, psp::JFT_PTR));
        let seg = bus.read16(linear(p, psp::JFT_PTR + 2));
        (linear(seg, off), bus.read16(linear(p, psp::JFT_SIZE)))
    }

    fn sft_index<B: Bus>(&self, bus: &mut B, handle: u16) -> R<usize> {
        let (jft, size) = self.jft(bus);
        if handle >= size {
            return Err(DosError::InvalidHandle);
        }
        let i = bus.read8(jft + handle as u32) as usize;
        match self.sft.get(i) {
            Some(Some(_)) => Ok(i),
            _ => Err(DosError::InvalidHandle),
        }
    }

    fn entry<B: Bus>(&mut self, bus: &mut B, handle: u16) -> R<&mut SftEntry> {
        let i = self.sft_index(bus, handle)?;
        Ok(self.sft[i].as_mut().expect("checked"))
    }

    fn free_handle<B: Bus>(&self, bus: &mut B) -> R<u16> {
        let (jft, size) = self.jft(bus);
        (0..size)
            .find(|&h| bus.read8(jft + h as u32) == JFT_FREE)
            .ok_or(DosError::TooManyOpenFiles)
    }

    /// Put `open` in the SFT and a free JFT slot; returns the handle.
    fn add_handle<B: Bus>(&mut self, bus: &mut B, open: Open, no_inherit: bool) -> R<u16> {
        let h = self.free_handle(bus)?;
        let i = self
            .sft
            .iter()
            .position(|e| e.is_none())
            .ok_or(DosError::TooManyOpenFiles)?;
        self.sft[i] = Some(SftEntry {
            refs: 1,
            open,
            no_inherit,
            raw: false,
        });
        let (jft, _) = self.jft(bus);
        bus.write8(jft + h as u32, i as u8);
        Ok(h)
    }

    pub fn close_handle<B: Bus>(&mut self, bus: &mut B, handle: u16) -> R {
        let i = self.sft_index(bus, handle)?;
        let (jft, _) = self.jft(bus);
        bus.write8(jft + handle as u32, JFT_FREE);
        let e = self.sft[i].as_mut().expect("checked");
        e.refs = e.refs.saturating_sub(1);
        if e.refs == 0 {
            self.sft[i] = None;
        }
        Ok(())
    }

    /// Create (or truncate, unless `exclusive`) the file at the guest path.
    fn create_path<B: Bus>(
        &mut self,
        bus: &mut B,
        raw: &[u8],
        attr: Attr,
        exclusive: bool,
    ) -> R<u16> {
        // Fail early so a full JFT doesn't leave an empty file behind.
        self.free_handle(bus)?;
        if let Some(dev) = device_name(raw) {
            return self.add_handle(bus, Open::Device(dev), false);
        }
        let (drive, path) = self.resolve_path(bus, raw)?;
        let d = self.drive_mut(drive)?;
        if exclusive && d.stat(&path).is_ok() {
            return Err(DosError::Exists);
        }
        let file = d.create(&path, attr)?;
        let mode = OpenMode::ReadWrite;
        self.add_handle(
            bus,
            Open::File {
                file,
                mode,
                drive,
                path,
            },
            false,
        )
    }

    fn open_mode<B: Bus>(
        &mut self,
        bus: &mut B,
        raw: &[u8],
        mode: OpenMode,
        no_inherit: bool,
    ) -> R<u16> {
        self.free_handle(bus)?;
        if let Some(dev) = device_name(raw) {
            return self.add_handle(bus, Open::Device(dev), no_inherit);
        }
        let (drive, path) = self.resolve_path(bus, raw)?;
        let file = self.drive_mut(drive)?.open(&path, mode)?;
        self.add_handle(
            bus,
            Open::File {
                file,
                mode,
                drive,
                path,
            },
            no_inherit,
        )
    }

    fn read_handle<B: Bus>(
        &mut self,
        bus: &mut B,
        handle: u16,
        seg: u16,
        off: u16,
        len: u16,
    ) -> R<u16> {
        let e = self.entry(bus, handle)?;
        let Open::File { file, mode, .. } = &mut e.open else {
            return Ok(0);
        };
        if !mode.can_read() {
            return Err(DosError::AccessDenied);
        }
        let mut buf = vec![0; len as usize];
        let mut n = 0;
        while n < buf.len() {
            match file.read(&mut buf[n..])? {
                0 => break,
                k => n += k,
            }
        }
        for (i, &b) in buf[..n].iter().enumerate() {
            bus.write8(linear(seg, off.wrapping_add(i as u16)), b);
        }
        Ok(n as u16)
    }

    fn write_handle<B: Bus>(
        &mut self,
        bus: &mut B,
        handle: u16,
        seg: u16,
        off: u16,
        len: u16,
    ) -> R<u16> {
        let buf: Vec<u8> = (0..len)
            .map(|i| bus.read8(linear(seg, off.wrapping_add(i))))
            .collect();
        self.write_bytes(bus, handle, &buf)
    }

    /// Device of `handle`, with its raw flag.
    pub(super) fn handle_device<B: Bus>(
        &mut self,
        bus: &mut B,
        handle: u16,
    ) -> Option<(Device, bool)> {
        match self.entry(bus, handle).ok()? {
            SftEntry {
                open: Open::Device(d),
                raw,
                ..
            } => Some((*d, *raw)),
            _ => None,
        }
    }

    /// Write `buf` to `handle` (file or device); CX=0 truncates files.
    pub(super) fn write_bytes<B: Bus>(&mut self, bus: &mut B, handle: u16, buf: &[u8]) -> R<u16> {
        let len = buf.len() as u16;
        let e = self.entry(bus, handle)?;
        let Open::File { file, mode, .. } = &mut e.open else {
            if let Open::Device(Device::Con) = e.open {
                self.con_write(bus, buf);
            }
            return Ok(len);
        };
        if !mode.can_write() {
            return Err(DosError::AccessDenied);
        }
        if len == 0 {
            let pos = file.seek(SeekFrom::Current(0))?;
            file.set_len(pos)?;
            return Ok(0);
        }
        let mut n = 0;
        while n < buf.len() {
            match file.write(&buf[n..])? {
                0 => break,
                k => n += k,
            }
        }
        Ok(n as u16)
    }

    fn dup_to<B: Bus>(&mut self, bus: &mut B, handle: u16, target: Option<u16>) -> R<u16> {
        let i = self.sft_index(bus, handle)?;
        let h = match target {
            Some(t) => {
                let (_, size) = self.jft(bus);
                if t >= size {
                    return Err(DosError::InvalidHandle);
                }
                if t == handle {
                    return Ok(t);
                }
                let _ = self.close_handle(bus, t);
                t
            }
            None => self.free_handle(bus)?,
        };
        let (jft, _) = self.jft(bus);
        bus.write8(jft + h as u32, i as u8);
        self.sft[i].as_mut().expect("checked").refs += 1;
        Ok(h)
    }

    fn stat_path<B: Bus>(&mut self, bus: &mut B, raw: &[u8]) -> R<Attr> {
        let (drive, path) = self.resolve_path(bus, raw)?;
        if path.is_root() {
            return Ok(Attr::DIRECTORY);
        }
        Ok(self.drive_mut(drive)?.stat(&path)?.attr)
    }

    /// INT 21h/3Bh: change the current directory of the path's drive.
    pub fn chdir<B: Bus>(&mut self, bus: &mut B, raw: &[u8]) -> R {
        let (drive, path) = self.resolve_path(bus, raw)?;
        if !self.is_dir(drive, &path) {
            return Err(DosError::PathNotFound);
        }
        self.cwd[drive as usize] = path;
        Ok(())
    }

    fn rmdir<B: Bus>(&mut self, bus: &mut B, raw: &[u8]) -> R {
        let (drive, path) = self.resolve_path(bus, raw)?;
        if path.is_root() {
            return Err(DosError::AccessDenied);
        }
        if self.cwd[drive as usize].starts_with(&path) {
            return Err(DosError::CurrentDirectory);
        }
        self.drive_mut(drive)?.rmdir(&path)
    }

    fn rename<B: Bus>(&mut self, bus: &mut B, from: &[u8], to: &[u8]) -> R {
        let (d1, p1) = self.resolve_path(bus, from)?;
        let (d2, p2) = self.resolve_path(bus, to)?;
        if d1 != d2 {
            return Err(DosError::NotSameDevice);
        }
        self.drive_mut(d1)?.rename(&p1, &p2)
    }

    /// INT 21h/44h subset: 00/01 device info, 06/07 status.
    fn ioctl<B: Bus>(&mut self, cpu: &mut Cpu, bus: &mut B, al: u8, h: u16) -> R {
        let dev = self.handle_device(bus, h);
        let drive = match &self.entry(bus, h)?.open {
            Open::File { drive, .. } => *drive as u16,
            Open::Device(_) => 0,
        };
        match al {
            0 => {
                let info = match dev {
                    Some((d, raw)) => d.info() | if raw { 0x20 } else { 0 },
                    None => drive & 0x3F,
                };
                cpu.regs.set_dx(info);
                cpu.regs.set_ax(info);
            }
            1 => {
                if cpu.regs.dh() != 0 {
                    return Err(DosError::InvalidFunction);
                }
                let raw = cpu.regs.dl() & 0x20 != 0;
                if dev.is_some() {
                    self.entry(bus, h)?.raw = raw;
                }
            }
            6 => {
                let ready = match dev {
                    Some((Device::Con, _)) => self.con_ready(bus),
                    Some(_) => false,
                    None => true,
                };
                cpu.regs.set_al(if ready { 0xFF } else { 0 });
            }
            7 => cpu.regs.set_al(0xFF),
            _ => return Err(DosError::InvalidFunction),
        }
        Ok(())
    }

    /// INT 21h/36h: (sectors per cluster, free clusters, bytes per sector,
    /// total clusters), scaled so the counts fit in 16 bits.
    fn free_space(&mut self, drive: u8) -> R<[u16; 4]> {
        let info = self.drive_mut(drive)?.info();
        const SECTOR: u64 = 512;
        let mut spc = 1u64;
        while info.total_bytes / (SECTOR * spc) > 0xFFFE && spc < 128 {
            spc *= 2;
        }
        let clusters = |b: u64| (b / (SECTOR * spc)).min(0xFFFE) as u16;
        Ok([
            spc as u16,
            clusters(info.free_bytes),
            SECTOR as u16,
            clusters(info.total_bytes),
        ])
    }

    /// File services of INT 21h; `None` if `ah` is not one of them.
    /// `Ok`/`Err` map to CF and AX by the caller.
    pub(super) fn file_fn<B: Bus>(&mut self, cpu: &mut Cpu, bus: &mut B, ah: u8) -> Option<R> {
        let ds = cpu.sregs.get(Seg::Ds);
        let r = &mut cpu.regs;
        let (al, bx, cx, dx) = (r.al(), r.bx(), r.cx(), r.dx());
        let path = |bus: &mut B| read_asciiz(bus, ds, dx);
        let res = match ah {
            0x0E => {
                if self.is_mounted(r.dl()) {
                    bus.write8(sda(SDA_DRIVE), r.dl());
                }
                r.set_al(26);
                return Some(Ok(()));
            }
            0x19 => {
                r.set_al(self.current_drive(bus));
                return Some(Ok(()));
            }
            0x36 => {
                match self.drive_arg(bus, r.dl()).and_then(|d| self.free_space(d)) {
                    Ok([a, b, c, d]) => {
                        cpu.regs.set_ax(a);
                        cpu.regs.set_bx(b);
                        cpu.regs.set_cx(c);
                        cpu.regs.set_dx(d);
                    }
                    Err(_) => cpu.regs.set_ax(0xFFFF),
                }
                return Some(Ok(()));
            }
            0x39 => {
                let p = path(bus);
                self.resolve_path(bus, &p)
                    .and_then(|(d, p)| self.drive_mut(d)?.mkdir(&p))
            }
            0x3A => {
                let p = path(bus);
                self.rmdir(bus, &p)
            }
            0x3B => {
                let p = path(bus);
                self.chdir(bus, &p)
            }
            0x3C | 0x5B => {
                let p = path(bus);
                self.create_path(bus, &p, Attr(cx as u8), ah == 0x5B)
                    .map(|h| cpu.regs.set_ax(h))
            }
            0x3D => {
                let p = path(bus);
                OpenMode::from_dos(al)
                    .ok_or(DosError::InvalidAccess)
                    .and_then(|m| self.open_mode(bus, &p, m, al & 0x80 != 0))
                    .map(|h| cpu.regs.set_ax(h))
            }
            0x3E => self.close_handle(bus, bx),
            0x3F => self
                .read_handle(bus, bx, ds, dx, cx)
                .map(|n| cpu.regs.set_ax(n)),
            0x40 => self
                .write_handle(bus, bx, ds, dx, cx)
                .map(|n| cpu.regs.set_ax(n)),
            0x41 => {
                let p = path(bus);
                self.resolve_path(bus, &p)
                    .and_then(|(d, p)| self.drive_mut(d)?.remove(&p))
            }
            0x42 => {
                let off = (cx as u32) << 16 | dx as u32;
                let pos = match al {
                    0 => Ok(SeekFrom::Start(off as u64)),
                    1 => Ok(SeekFrom::Current(off as i32 as i64)),
                    2 => Ok(SeekFrom::End(off as i32 as i64)),
                    _ => Err(DosError::InvalidFunction),
                };
                pos.and_then(|pos| match &mut self.entry(bus, bx)?.open {
                    Open::File { file, .. } => file.seek(pos),
                    Open::Device(_) => Ok(0),
                })
                .map(|p| {
                    cpu.regs.set_ax(p as u16);
                    cpu.regs.set_dx((p >> 16) as u16);
                })
            }
            0x43 => {
                let p = path(bus);
                match al {
                    0 => self.stat_path(bus, &p).map(|a| cpu.regs.set_cx(a.0 as u16)),
                    1 => self
                        .resolve_path(bus, &p)
                        .and_then(|(d, p)| self.drive_mut(d)?.set_attr(&p, Attr(cx as u8))),
                    _ => Err(DosError::InvalidFunction),
                }
            }
            0x44 => self.ioctl(cpu, bus, al, bx),
            0x45 => self.dup_to(bus, bx, None).map(|h| cpu.regs.set_ax(h)),
            0x46 => self.dup_to(bus, bx, Some(cx)).map(|_| ()),
            0x47 => {
                let si = r.si();
                self.drive_arg(bus, r.dl()).map(|d| {
                    let s = self.cwd[d as usize].to_bytes();
                    write_asciiz(bus, ds, si, &s[1..]);
                    cpu.regs.set_ax(0x0100);
                })
            }
            0x56 => {
                let from = path(bus);
                let to = read_asciiz(bus, cpu.sregs.get(Seg::Es), cpu.regs.di());
                self.rename(bus, &from, &to)
            }
            0x57 => match &mut self.entry(bus, bx).map(|e| &mut e.open) {
                Err(e) => Err(*e),
                Ok(Open::Device(_)) => Ok(()),
                Ok(Open::File { file, .. }) => match al {
                    0 => {
                        let (date, time) = file.mtime().to_dos();
                        cpu.regs.set_cx(time);
                        cpu.regs.set_dx(date);
                        Ok(())
                    }
                    1 => file.set_mtime(DosDateTime::from_dos(dx, cx)),
                    _ => Err(DosError::InvalidFunction),
                },
            },
            _ => return None,
        };
        Some(res)
    }
}
