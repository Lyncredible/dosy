//! Host directory drive (and single-file mounts) with 8.3 name mapping.
//!
//! Every lookup lists the host directory and maps names with
//! [`shortname::map_dir`], so lookup is case-insensitive on every host and
//! names stay consistent with `read_dir`. Paths never leave the mount root:
//! `DosPath` has no `..`, and each resolved host path is canonicalized and
//! checked against the root, which catches symlinks pointing outside.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::drive::dos_seek;
use crate::{
    Attr, DirEntry, DosDateTime, DosError, DosFile, DosName, DosPath, Drive, DriveInfo, OpenMode,
    SeekFrom, shortname,
};

/// Reported sizes (host free space isn't queried; DOS only needs a
/// plausible value).
const TOTAL_BYTES: u64 = 2 << 30;
const FREE_BYTES: u64 = 1 << 30;
const MAX_FILE: u64 = u32::MAX as u64;

/// Options for [`open_mount`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MountOptions {
    pub read_only: bool,
    /// Volume label (up to 11 chars).
    pub label: String,
}

/// Mount a host directory, or a single file as a drive holding just that
/// file. (Zip archives are added by task 028.)
pub fn open_mount(path: &Path, opts: &MountOptions) -> io::Result<Box<dyn Drive>> {
    let meta = fs::metadata(path)?;
    if meta.is_dir() {
        Ok(Box::new(HostDrive::new(path, opts)?))
    } else if meta.is_file() {
        Ok(Box::new(HostDrive::single_file(path, opts)?))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a file or directory",
        ))
    }
}

fn io_err(e: io::Error) -> DosError {
    match e.kind() {
        io::ErrorKind::NotFound => DosError::FileNotFound,
        _ => DosError::AccessDenied,
    }
}

// --- Time: host `SystemTime` (UTC) <-> DOS local time. ---

/// Days since 1970-01-01 → (year, month, day). (H. Hinnant's algorithm.)
fn civil_from_days(z: i64) -> (i64, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

fn days_from_civil(y: i64, m: u8, d: u8) -> i64 {
    let y = y - (m <= 2) as i64;
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Host time as a DOS timestamp (UTC; clamped to 1980–2107).
pub fn to_dos_time(t: SystemTime) -> DosDateTime {
    let secs = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => return DosDateTime::default(),
    };
    let (y, mo, d) = civil_from_days(secs.div_euclid(86_400));
    if y < 1980 {
        return DosDateTime::default();
    }
    if y > 2107 {
        return DosDateTime::from_dos(0xFF9F, 0xBF7D); // 2107-12-31 23:59:58
    }
    let s = secs.rem_euclid(86_400);
    DosDateTime {
        year: y as u16,
        month: mo,
        day: d,
        hour: (s / 3600) as u8,
        minute: (s / 60 % 60) as u8,
        second: (s % 60) as u8,
    }
}

pub fn from_dos_time(t: DosDateTime) -> SystemTime {
    let days = days_from_civil(t.year as i64, t.month.clamp(1, 12), t.day.clamp(1, 31));
    let secs = days * 86_400 + t.hour as i64 * 3600 + t.minute as i64 * 60 + t.second as i64;
    UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64)
}

/// A host directory (or one file) as a DOS drive.
pub struct HostDrive {
    /// Canonical mount root.
    root: PathBuf,
    /// Single-file mount: the only host name visible at the root.
    only: Option<String>,
    read_only: bool,
    label: String,
}

impl HostDrive {
    pub fn new(dir: impl AsRef<Path>, opts: &MountOptions) -> io::Result<Self> {
        let root = fs::canonicalize(dir)?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a directory",
            ));
        }
        Ok(Self {
            root,
            only: None,
            read_only: opts.read_only,
            label: opts.label.clone(),
        })
    }

    /// A drive whose root holds only `file`; no other files can be created.
    pub fn single_file(file: impl AsRef<Path>, opts: &MountOptions) -> io::Result<Self> {
        let path = fs::canonicalize(file)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad file name"))?
            .to_string();
        let mut d = Self::new(path.parent().unwrap_or(&path), opts)?;
        d.only = Some(name);
        Ok(d)
    }

    fn writable(&self) -> Result<(), DosError> {
        if self.read_only {
            Err(DosError::AccessDenied)
        } else {
            Ok(())
        }
    }

    /// Short-name map of host directory `dir`.
    fn list(&self, dir: &Path) -> Result<Vec<(DosName, String)>, DosError> {
        let names: Vec<String> = fs::read_dir(dir)
            .map_err(|_| DosError::PathNotFound)?
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| dir != self.root || self.only.as_ref().is_none_or(|o| o == n))
            .collect();
        Ok(shortname::map_dir(&names))
    }

    /// Reject host paths that resolve outside the root (symlinks).
    fn inside(&self, path: &Path) -> Result<(), DosError> {
        match fs::canonicalize(path) {
            Ok(c) if c.starts_with(&self.root) => Ok(()),
            Ok(_) => Err(DosError::AccessDenied),
            Err(_) => Err(DosError::FileNotFound),
        }
    }

    /// Host path of directory `p`.
    fn host_dir(&self, p: &DosPath) -> Result<PathBuf, DosError> {
        let mut cur = self.root.clone();
        for c in p.components() {
            let host = self
                .list(&cur)?
                .into_iter()
                .find(|(n, _)| n == c)
                .ok_or(DosError::PathNotFound)?
                .1;
            cur.push(host);
            self.inside(&cur).map_err(|_| DosError::PathNotFound)?;
            if !cur.is_dir() {
                return Err(DosError::PathNotFound);
            }
        }
        Ok(cur)
    }

    /// Host path for `p`: `(path, exists)`. A missing final component maps
    /// to its DOS name in the (existing) parent directory.
    fn target(&self, p: &DosPath) -> Result<(PathBuf, bool), DosError> {
        let (parent, name) = match (p.parent(), p.file_name()) {
            (Some(parent), Some(name)) => (parent, *name),
            _ => return Err(DosError::AccessDenied), // the root
        };
        let dir = self.host_dir(&parent)?;
        match self.list(&dir)?.into_iter().find(|(n, _)| *n == name) {
            Some((_, host)) => {
                let path = dir.join(host);
                self.inside(&path)?;
                Ok((path, true))
            }
            None => Ok((dir.join(name.to_string()), false)),
        }
    }

    fn existing(&self, p: &DosPath) -> Result<PathBuf, DosError> {
        match self.target(p)? {
            (path, true) => Ok(path),
            _ => Err(DosError::FileNotFound),
        }
    }

    /// May a new entry be created at `path`? (Single-file mounts: no.)
    fn can_create(&self, exists: bool) -> Result<(), DosError> {
        self.writable()?;
        if self.only.is_some() && !exists {
            return Err(DosError::AccessDenied);
        }
        Ok(())
    }
}

fn entry(name: DosName, meta: &fs::Metadata) -> DirEntry {
    let attr = if meta.is_dir() {
        Attr::DIRECTORY
    } else if meta.permissions().readonly() {
        Attr::ARCHIVE | Attr::READ_ONLY
    } else {
        Attr::ARCHIVE
    };
    DirEntry {
        name,
        attr,
        size: if meta.is_dir() {
            0
        } else {
            meta.len().min(MAX_FILE) as u32
        },
        mtime: meta.modified().map(to_dos_time).unwrap_or_default(),
    }
}

fn set_read_only(path: &Path, ro: bool) -> io::Result<()> {
    let mut perm = fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = perm.mode();
        perm.set_mode(if ro { mode & !0o222 } else { mode | 0o200 });
    }
    #[cfg(not(unix))]
    perm.set_readonly(ro);
    fs::set_permissions(path, perm)
}

impl Drive for HostDrive {
    fn info(&self) -> DriveInfo {
        DriveInfo {
            label: self.label.clone(),
            read_only: self.read_only,
            total_bytes: TOTAL_BYTES,
            free_bytes: if self.read_only { 0 } else { FREE_BYTES },
        }
    }

    fn open(&mut self, p: &DosPath, mode: OpenMode) -> Result<Box<dyn DosFile>, DosError> {
        let path = self.existing(p)?;
        if path.is_dir() {
            return Err(DosError::AccessDenied);
        }
        if mode.can_write() {
            self.writable()?;
        }
        let file = OpenOptions::new()
            .read(mode.can_read())
            .write(mode.can_write())
            .open(&path)
            .map_err(io_err)?;
        Ok(Box::new(HostFile {
            file,
            path,
            pos: 0,
            mode,
        }))
    }

    fn create(&mut self, p: &DosPath, _attr: Attr) -> Result<Box<dyn DosFile>, DosError> {
        let (path, exists) = self.target(p)?;
        self.can_create(exists)?;
        if exists
            && (path.is_dir() || fs::metadata(&path).is_ok_and(|m| m.permissions().readonly()))
        {
            return Err(DosError::AccessDenied);
        }
        // A new name must not resolve to an existing host file (e.g. via an
        // NTFS 8.3 alias that differs from ours, or a symlink): create_new.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(exists)
            .truncate(exists)
            .create_new(!exists)
            .open(&path)
            .map_err(io_err)?;
        Ok(Box::new(HostFile {
            file,
            path,
            pos: 0,
            mode: OpenMode::ReadWrite,
        }))
    }

    fn stat(&mut self, p: &DosPath) -> Result<DirEntry, DosError> {
        let name = *p.file_name().ok_or(DosError::FileNotFound)?;
        let meta = fs::metadata(self.existing(p)?).map_err(io_err)?;
        Ok(entry(name, &meta))
    }

    fn read_dir(&mut self, dir: &DosPath) -> Result<Vec<DirEntry>, DosError> {
        let host = self.host_dir(dir)?;
        Ok(self
            .list(&host)?
            .into_iter()
            .filter_map(|(n, h)| {
                let path = host.join(h);
                self.inside(&path).ok()?;
                Some(entry(n, &fs::metadata(&path).ok()?))
            })
            .collect())
    }

    fn remove(&mut self, p: &DosPath) -> Result<(), DosError> {
        self.writable()?;
        let path = self.existing(p)?;
        let meta = fs::metadata(&path).map_err(io_err)?;
        if meta.is_dir() || meta.permissions().readonly() {
            return Err(DosError::AccessDenied);
        }
        fs::remove_file(path).map_err(io_err)
    }

    fn rename(&mut self, from: &DosPath, to: &DosPath) -> Result<(), DosError> {
        self.writable()?;
        let src = self.existing(from)?;
        let (dst, exists) = self.target(to)?;
        self.can_create(exists)?;
        // `symlink_metadata`: the unmapped host name must not already exist
        // (NTFS 8.3 alias), or the rename would replace it.
        if exists || fs::symlink_metadata(&dst).is_ok() || (src.is_dir() && to.starts_with(from)) {
            return Err(DosError::AccessDenied);
        }
        fs::rename(src, dst).map_err(io_err)
    }

    fn mkdir(&mut self, p: &DosPath) -> Result<(), DosError> {
        let (path, exists) = self.target(p)?;
        self.can_create(exists)?;
        if exists {
            return Err(DosError::AccessDenied);
        }
        fs::create_dir(path).map_err(io_err)
    }

    fn rmdir(&mut self, p: &DosPath) -> Result<(), DosError> {
        self.writable()?;
        let path = match self.existing(p) {
            Err(DosError::FileNotFound) => return Err(DosError::PathNotFound),
            r => r?,
        };
        if !path.is_dir() {
            return Err(DosError::PathNotFound);
        }
        fs::remove_dir(path).map_err(io_err)
    }

    fn set_attr(&mut self, p: &DosPath, attr: Attr) -> Result<(), DosError> {
        self.writable()?;
        let path = self.existing(p)?;
        if path.is_dir() {
            return Ok(());
        }
        set_read_only(&path, attr.contains(Attr::READ_ONLY)).map_err(io_err)
    }
}

struct HostFile {
    file: File,
    path: PathBuf,
    /// DOS position (32-bit, may lie past EOF).
    pos: u64,
    mode: OpenMode,
}

impl HostFile {
    fn len(&self) -> Result<u64, DosError> {
        self.file.metadata().map(|m| m.len()).map_err(io_err)
    }
}

impl DosFile for HostFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, DosError> {
        if !self.mode.can_read() {
            return Err(DosError::AccessDenied);
        }
        if self.pos >= self.len()? {
            return Ok(0);
        }
        self.file
            .seek(io::SeekFrom::Start(self.pos))
            .map_err(io_err)?;
        let mut n = 0;
        while n < buf.len() {
            match self.file.read(&mut buf[n..]).map_err(io_err)? {
                0 => break,
                k => n += k,
            }
        }
        self.pos += n as u64;
        Ok(n)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, DosError> {
        if !self.mode.can_write() {
            return Err(DosError::AccessDenied);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        if self.pos + buf.len() as u64 > MAX_FILE {
            return Err(DosError::InsufficientMemory);
        }
        self.file
            .seek(io::SeekFrom::Start(self.pos))
            .map_err(io_err)?;
        self.file.write_all(buf).map_err(io_err)?;
        self.pos += buf.len() as u64;
        Ok(buf.len())
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, DosError> {
        self.pos = dos_seek(self.pos, self.len()?, pos);
        Ok(self.pos)
    }

    fn set_len(&mut self, len: u64) -> Result<(), DosError> {
        if !self.mode.can_write() {
            return Err(DosError::AccessDenied);
        }
        if len > MAX_FILE {
            return Err(DosError::InsufficientMemory);
        }
        self.file.set_len(len).map_err(io_err)
    }

    fn mtime(&self) -> DosDateTime {
        self.file
            .metadata()
            .and_then(|m| m.modified())
            .map(to_dos_time)
            .unwrap_or_default()
    }

    fn set_mtime(&mut self, t: DosDateTime) -> Result<(), DosError> {
        let t = from_dos_time(t);
        if self.file.set_modified(t).is_ok() {
            return Ok(());
        }
        // Some hosts need write access to set times on a read-only handle.
        OpenOptions::new()
            .write(true)
            .open(&self.path)
            .and_then(|f| f.set_modified(t))
            .map_err(io_err)
    }
}

#[cfg(test)]
mod tests;
