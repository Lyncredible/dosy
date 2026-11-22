//! `Drive` / `DosFile` traits and the value types they exchange.

use std::fmt;
use std::ops::{BitAnd, BitOr, BitOrAssign, Not};

pub use std::io::SeekFrom;

use crate::{DosError, DosName, DosPath};

/// DOS file attribute byte.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Attr(pub u8);

impl Attr {
    pub const NONE: Self = Self(0);
    pub const READ_ONLY: Self = Self(0x01);
    pub const HIDDEN: Self = Self(0x02);
    pub const SYSTEM: Self = Self(0x04);
    pub const VOLUME: Self = Self(0x08);
    pub const DIRECTORY: Self = Self(0x10);
    pub const ARCHIVE: Self = Self(0x20);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl BitOr for Attr {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl BitOrAssign for Attr {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}
impl BitAnd for Attr {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl Not for Attr {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

impl fmt::Debug for Attr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Attr({:#04x})", self.0)
    }
}

/// Local date and time with DOS's 2-second resolution (1980–2107).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DosDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Default for DosDateTime {
    /// 1980-01-01 00:00:00, the DOS epoch.
    fn default() -> Self {
        Self {
            year: 1980,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        }
    }
}

impl DosDateTime {
    /// Packed `(date, time)` words as in directory entries and INT 21h/57h.
    pub fn to_dos(self) -> (u16, u16) {
        let date = (self.year.saturating_sub(1980).min(127) << 9)
            | (self.month as u16 & 0xF) << 5
            | (self.day as u16 & 0x1F);
        let time = (self.hour as u16 & 0x1F) << 11
            | (self.minute as u16 & 0x3F) << 5
            | ((self.second as u16 / 2) & 0x1F);
        (date, time)
    }

    pub fn from_dos(date: u16, time: u16) -> Self {
        Self {
            year: 1980 + (date >> 9),
            month: (date >> 5 & 0xF) as u8,
            day: (date & 0x1F) as u8,
            hour: (time >> 11) as u8,
            minute: (time >> 5 & 0x3F) as u8,
            second: (time & 0x1F) as u8 * 2,
        }
    }
}

/// Access mode (INT 21h/3Dh AL bits 0–2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OpenMode {
    Read,
    Write,
    ReadWrite,
}

impl OpenMode {
    pub fn from_dos(al: u8) -> Option<Self> {
        match al & 7 {
            0 => Some(Self::Read),
            1 => Some(Self::Write),
            2 => Some(Self::ReadWrite),
            _ => None,
        }
    }
    pub fn can_read(self) -> bool {
        self != Self::Write
    }
    pub fn can_write(self) -> bool {
        self != Self::Read
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveInfo {
    /// Volume label (up to 11 chars), empty for none.
    pub label: String,
    pub read_only: bool,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: DosName,
    pub attr: Attr,
    pub size: u32,
    pub mtime: DosDateTime,
}

impl DirEntry {
    pub fn is_dir(&self) -> bool {
        self.attr.contains(Attr::DIRECTORY)
    }
}

/// A mounted DOS drive. Paths are drive-relative and already resolved; the
/// kernel owns drive letters, current directories and FindFirst state.
///
/// Error conventions: missing final component → `FileNotFound`, missing
/// parent → `PathNotFound`, writes to a read-only drive or file, existing
/// targets and non-empty directories → `AccessDenied`.
pub trait Drive: Send {
    fn info(&self) -> DriveInfo;
    /// Open an existing file (not a directory).
    fn open(&mut self, p: &DosPath, mode: OpenMode) -> Result<Box<dyn DosFile>, DosError>;
    /// Create or truncate a file, opened read/write.
    fn create(&mut self, p: &DosPath, attr: Attr) -> Result<Box<dyn DosFile>, DosError>;
    fn stat(&mut self, p: &DosPath) -> Result<DirEntry, DosError>;
    /// Entries of a directory, excluding `.` and `..`, in a stable order.
    fn read_dir(&mut self, dir: &DosPath) -> Result<Vec<DirEntry>, DosError>;
    /// Delete a file (not a directory).
    fn remove(&mut self, p: &DosPath) -> Result<(), DosError>;
    /// Rename or move a file or directory within the drive.
    fn rename(&mut self, from: &DosPath, to: &DosPath) -> Result<(), DosError>;
    fn mkdir(&mut self, p: &DosPath) -> Result<(), DosError>;
    /// Remove an empty directory.
    fn rmdir(&mut self, p: &DosPath) -> Result<(), DosError>;
    /// Set file attributes (INT 21h/4301h); `DIRECTORY`/`VOLUME` are ignored.
    fn set_attr(&mut self, p: &DosPath, attr: Attr) -> Result<(), DosError>;
}

/// An open file. Positions may go past EOF; writing there zero-fills.
pub trait DosFile: Send {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, DosError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, DosError>;
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, DosError>;
    fn set_len(&mut self, len: u64) -> Result<(), DosError>;
    fn mtime(&self) -> DosDateTime;
    /// INT 21h/5701h.
    fn set_mtime(&mut self, t: DosDateTime) -> Result<(), DosError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datetime_packs_like_dos() {
        let t = DosDateTime {
            year: 1994,
            month: 12,
            day: 10,
            hour: 23,
            minute: 59,
            second: 58,
        };
        let (d, tm) = t.to_dos();
        assert_eq!(d, (14 << 9) | (12 << 5) | 10);
        assert_eq!(tm, (23 << 11) | (59 << 5) | 29);
        assert_eq!(DosDateTime::from_dos(d, tm), t);
        assert_eq!(DosDateTime::default().to_dos(), (0x21, 0));
    }

    #[test]
    fn attrs_and_modes() {
        let a = Attr::READ_ONLY | Attr::ARCHIVE;
        assert!(a.contains(Attr::READ_ONLY) && !a.contains(Attr::HIDDEN));
        assert_eq!((a & !Attr::READ_ONLY), Attr::ARCHIVE);
        assert_eq!(OpenMode::from_dos(0x42), Some(OpenMode::ReadWrite));
        assert_eq!(OpenMode::from_dos(3), None);
        assert!(OpenMode::Write.can_write() && !OpenMode::Write.can_read());
    }
}
