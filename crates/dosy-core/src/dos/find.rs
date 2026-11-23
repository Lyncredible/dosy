//! INT 21h/4Eh–4Fh find first/next.
//!
//! DTA layout: `+00` drive, `+01` 11-byte FCB pattern, `+0C` search
//! attributes, `+0D` next entry index (word), `+0F` search directory id
//! (word, into `Dos::search_dirs`), `+15` attr, `+16` time, `+18` date,
//! `+1A` size (dword), `+1E` ASCIIZ name (13 bytes).

use super::Dos;
use super::path::read_asciiz;
use crate::bus::{Bus, linear};
use crate::cpu::{Cpu, Seg};
use crate::vfs::{Attr, DirEntry, DosError, DosPath, DosPattern};

const D_DRIVE: u16 = 0x00;
const D_PATTERN: u16 = 0x01;
const D_ATTR: u16 = 0x0C;
const D_INDEX: u16 = 0x0D;
const D_DIR: u16 = 0x0F;
const D_FOUND_ATTR: u16 = 0x15;
const D_TIME: u16 = 0x16;
const D_DATE: u16 = 0x18;
const D_SIZE: u16 = 0x1A;
const D_NAME: u16 = 0x1E;

/// Hidden, system and directory entries are only listed when asked for.
const SPECIAL: u8 = Attr::HIDDEN.0 | Attr::SYSTEM.0 | Attr::DIRECTORY.0;

fn wanted(e: &DirEntry, attr: u8, pat: &DosPattern) -> bool {
    !e.attr.contains(Attr::VOLUME) && e.attr.0 & SPECIAL & !attr == 0 && pat.matches(&e.name)
}

impl Dos {
    /// Stable id for a search directory (ids are reused per path).
    fn search_dir_id(&mut self, drive: u8, dir: DosPath) -> u16 {
        let key = (drive, dir);
        match self.search_dirs.iter().position(|d| *d == key) {
            Some(i) => i as u16,
            None => {
                self.search_dirs.push(key);
                (self.search_dirs.len() - 1) as u16
            }
        }
    }

    /// Scan from `start`; fill the DTA with the first match.
    fn find_from<B: Bus>(&mut self, bus: &mut B, start: u16) -> Result<(), DosError> {
        let (seg, off) = self.dta(bus);
        let at = |o: u16| linear(seg, off.wrapping_add(o));
        let mut fcb = [0u8; 11];
        for (i, b) in fcb.iter_mut().enumerate() {
            *b = bus.read8(at(D_PATTERN + i as u16));
        }
        let pat = DosPattern::from_fcb(fcb);
        let attr = bus.read8(at(D_ATTR));
        let id = bus.read16(at(D_DIR)) as usize;
        let (drive, dir) = self
            .search_dirs
            .get(id)
            .cloned()
            .ok_or(DosError::NoMoreFiles)?;
        if attr == Attr::VOLUME.0 {
            return Err(DosError::NoMoreFiles); // volume labels are not listed
        }
        let entries = self.drive_mut(drive)?.read_dir(&dir).map_err(|e| match e {
            DosError::FileNotFound => DosError::PathNotFound,
            e => e,
        })?;
        let found = entries
            .iter()
            .enumerate()
            .skip(start as usize)
            .find(|(_, e)| wanted(e, attr, &pat));
        let Some((i, e)) = found else {
            return Err(DosError::NoMoreFiles);
        };
        bus.write16(at(D_INDEX), i as u16 + 1);
        bus.write8(at(D_FOUND_ATTR), e.attr.0);
        let (date, time) = e.mtime.to_dos();
        bus.write16(at(D_TIME), time);
        bus.write16(at(D_DATE), date);
        bus.write32(at(D_SIZE), e.size);
        let name = e.name.to_string();
        for i in 0..13u16 {
            let b = name.as_bytes().get(i as usize).copied().unwrap_or(0);
            bus.write8(at(D_NAME + i), b);
        }
        Ok(())
    }

    pub(super) fn find_first<B: Bus>(
        &mut self,
        cpu: &mut Cpu,
        bus: &mut B,
    ) -> Result<(), DosError> {
        let raw = read_asciiz(bus, cpu.sregs.get(Seg::Ds), cpu.regs.dx());
        let r = self.resolve(bus, &raw)?;
        let (dir, pat) = r.dir_and_pattern()?;
        if !self.is_dir(r.drive, &dir) {
            return Err(DosError::PathNotFound);
        }
        let id = self.search_dir_id(r.drive, dir);
        let (seg, off) = self.dta(bus);
        let at = |o: u16| linear(seg, off.wrapping_add(o));
        bus.write8(at(D_DRIVE), r.drive);
        for (i, &b) in pat.fcb().iter().enumerate() {
            bus.write8(at(D_PATTERN + i as u16), b);
        }
        bus.write8(at(D_ATTR), cpu.regs.cl());
        bus.write16(at(D_DIR), id);
        self.find_from(bus, 0)
    }

    pub(super) fn find_next<B: Bus>(&mut self, bus: &mut B) -> Result<(), DosError> {
        let (seg, off) = self.dta(bus);
        let start = bus.read16(linear(seg, off.wrapping_add(D_INDEX)));
        self.find_from(bus, start)
    }
}
