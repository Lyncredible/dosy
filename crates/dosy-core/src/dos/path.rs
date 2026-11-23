//! Drive letters, current directories and guest path resolution.

use super::{Dos, SDA_DRIVE, sda};
use crate::bus::{Bus, linear};
use crate::vfs::{DosError, DosName, DosPath, DosPattern};

/// Longest ASCIIZ path read from the guest.
const MAX_PATH: u16 = 128;

/// Read an ASCIIZ string at `seg:off` (offset wraps in the segment).
pub fn read_asciiz<B: Bus>(bus: &mut B, seg: u16, off: u16) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..MAX_PATH {
        match bus.read8(linear(seg, off.wrapping_add(i))) {
            0 => break,
            b => out.push(b),
        }
    }
    out
}

/// Write `s` plus a NUL at `seg:off`.
pub fn write_asciiz<B: Bus>(bus: &mut B, seg: u16, off: u16, s: &[u8]) {
    for (i, &b) in s.iter().chain(&[0]).enumerate() {
        bus.write8(linear(seg, off.wrapping_add(i as u16)), b);
    }
}

/// Truncate an over-long `NAME.EXT` component to 8.3, as DOS does
/// (`LONGFILENAME.TEXT` → `LONGFILE.TEX`).
fn truncate_83(comp: &[u8]) -> Vec<u8> {
    match comp.iter().position(|&c| c == b'.') {
        Some(i) => {
            let (base, ext) = (&comp[..i], &comp[i + 1..]);
            let mut out = base[..base.len().min(8)].to_vec();
            out.push(b'.');
            out.extend(&ext[..ext.len().min(3)]);
            out
        }
        None => comp[..comp.len().min(8)].to_vec(),
    }
}

/// A resolved guest path: drive index plus normalized components as guest
/// bytes (the last one may still hold wildcards).
pub struct Resolved {
    pub drive: u8,
    pub parts: Vec<Vec<u8>>,
}

impl Resolved {
    fn names(parts: &[Vec<u8>]) -> Result<DosPath, DosError> {
        parts
            .iter()
            .map(|p| DosName::from_bytes(p))
            .collect::<Result<Vec<_>, _>>()
            .map(DosPath::from_names)
    }

    pub fn path(&self) -> Result<DosPath, DosError> {
        Self::names(&self.parts)
    }

    /// Directory plus a FindFirst pattern for the last component.
    pub fn dir_and_pattern(&self) -> Result<(DosPath, DosPattern), DosError> {
        let (last, dir) = self.parts.split_last().ok_or(DosError::NoMoreFiles)?;
        Ok((Self::names(dir)?, DosPattern::from_bytes(last)?))
    }
}

impl Dos {
    pub fn current_drive<B: Bus>(&self, bus: &mut B) -> u8 {
        bus.read8(sda(SDA_DRIVE))
    }

    /// Select the current drive (0 = A:); no check that it is mounted.
    pub fn set_current_drive<B: Bus>(&self, bus: &mut B, drive: u8) {
        bus.write8(sda(SDA_DRIVE), drive);
    }

    pub fn is_mounted(&self, drive: u8) -> bool {
        self.drives.get(drive as usize).is_some_and(|d| d.is_some())
    }

    /// Drive from a DL-style number (0 = default, 1 = A:, ...).
    pub fn drive_arg<B: Bus>(&self, bus: &mut B, dl: u8) -> Result<u8, DosError> {
        let d = match dl {
            0 => self.current_drive(bus),
            n => n - 1,
        };
        if self.is_mounted(d) {
            Ok(d)
        } else {
            Err(DosError::InvalidDrive)
        }
    }

    /// Resolve a guest path against the current drive and directory.
    pub fn resolve<B: Bus>(&self, bus: &mut B, raw: &[u8]) -> Result<Resolved, DosError> {
        // Code-page bytes (>= 80h) pass through to the names.
        let (drive, rest) = match raw {
            [l, b':', rest @ ..] if l.is_ascii_alphabetic() => {
                (l.to_ascii_uppercase() - b'A', rest)
            }
            _ => (self.current_drive(bus), raw),
        };
        if !self.is_mounted(drive) {
            return Err(DosError::PathNotFound);
        }
        let sep = |c: &u8| *c == b'\\' || *c == b'/';
        let mut parts: Vec<Vec<u8>> = if rest.first().is_some_and(sep) {
            Vec::new()
        } else {
            self.cwd[drive as usize]
                .components()
                .iter()
                .map(|n| n.to_bytes())
                .collect()
        };
        for comp in rest.split(sep) {
            match comp {
                b"" | b"." => {}
                b".." => {
                    parts.pop().ok_or(DosError::PathNotFound)?;
                }
                c => parts.push(truncate_83(c)),
            }
        }
        Ok(Resolved { drive, parts })
    }

    /// Resolve to a drive and a validated path.
    pub fn resolve_path<B: Bus>(&self, bus: &mut B, raw: &[u8]) -> Result<(u8, DosPath), DosError> {
        let r = self.resolve(bus, raw)?;
        Ok((r.drive, r.path()?))
    }

    pub fn drive_mut(&mut self, drive: u8) -> Result<&mut dyn crate::vfs::Drive, DosError> {
        match self.drives.get_mut(drive as usize) {
            Some(Some(d)) => Ok(d.as_mut()),
            _ => Err(DosError::InvalidDrive),
        }
    }

    /// True if `p` is the root or an existing directory.
    pub fn is_dir(&mut self, drive: u8, p: &DosPath) -> bool {
        p.is_root()
            || self
                .drive_mut(drive)
                .and_then(|d| d.stat(p))
                .is_ok_and(|e| e.is_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_83;

    #[test]
    fn long_names_truncate() {
        assert_eq!(truncate_83(b"LONGFILENAME.TEXT"), b"LONGFILE.TEX");
        assert_eq!(truncate_83(b"ABCDEFGHIJ"), b"ABCDEFGH");
        assert_eq!(truncate_83(b"A.B"), b"A.B");
        assert_eq!(truncate_83(b"FOO."), b"FOO.");
        assert_eq!(truncate_83(b"*.*"), b"*.*");
    }
}
