//! Writable overlay over a read-only drive (e.g. a zip).
//!
//! Reads fall through to `lower` unless `upper` has the path. Writes copy the
//! file up to `upper` first; deleting or renaming a lower entry records a
//! whiteout so it disappears from the merged view. Whiteouts live in memory
//! (they are not persisted with a directory upper). The lower drive is
//! never written.

use std::collections::{BTreeMap, HashSet};

use crate::{Attr, DirEntry, DosError, DosFile, DosPath, Drive, DriveInfo, OpenMode};

pub struct OverlayDrive<L: Drive, U: Drive> {
    lower: L,
    upper: U,
    whiteouts: HashSet<DosPath>,
}

impl<L: Drive, U: Drive> OverlayDrive<L, U> {
    pub fn new(lower: L, upper: U) -> Self {
        Self {
            lower,
            upper,
            whiteouts: HashSet::new(),
        }
    }

    /// The upper (written) layer.
    pub fn upper(&self) -> &U {
        &self.upper
    }

    fn whited(&self, p: &DosPath) -> bool {
        let mut cur = Some(p.clone());
        while let Some(c) = cur {
            if self.whiteouts.contains(&c) {
                return true;
            }
            cur = c.parent();
        }
        false
    }

    fn in_upper(&mut self, p: &DosPath) -> Option<DirEntry> {
        self.upper.stat(p).ok()
    }

    fn in_lower(&mut self, p: &DosPath) -> Option<DirEntry> {
        if self.whited(p) {
            return None;
        }
        self.lower.stat(p).ok()
    }

    fn lookup(&mut self, p: &DosPath) -> Option<DirEntry> {
        self.in_upper(p).or_else(|| self.in_lower(p))
    }

    fn is_dir(&mut self, p: &DosPath) -> bool {
        p.is_root() || self.lookup(p).is_some_and(|e| e.is_dir())
    }

    /// Error for a missing `p`: its parent missing → path not found.
    fn missing(&mut self, p: &DosPath) -> DosError {
        match p.parent() {
            Some(parent) if !self.is_dir(&parent) => DosError::PathNotFound,
            _ => DosError::FileNotFound,
        }
    }

    /// Make every directory of `dir` exist in the upper layer.
    fn upper_dirs(&mut self, dir: &DosPath) -> Result<(), DosError> {
        let mut cur = DosPath::root();
        for c in dir.components() {
            cur = cur.join(*c);
            match self.upper.stat(&cur) {
                Ok(e) if e.is_dir() => {}
                Ok(_) => return Err(DosError::PathNotFound),
                Err(_) => {
                    if !self.is_dir(&cur) {
                        return Err(DosError::PathNotFound);
                    }
                    self.upper.mkdir(&cur)?;
                }
            }
        }
        Ok(())
    }

    /// Copy a lower file (or a whole directory tree) up.
    fn copy_up(&mut self, p: &DosPath) -> Result<(), DosError> {
        if self.in_upper(p).is_some() && !self.is_dir(p) {
            return Ok(());
        }
        let e = match self.lookup(p) {
            Some(e) => e,
            None => return Err(self.missing(p)),
        };
        self.upper_dirs(&p.parent().unwrap_or_default())?;
        if e.is_dir() {
            if self.in_upper(p).is_none() {
                self.upper.mkdir(p)?;
            }
            for child in self.merged(p)? {
                self.copy_up(&p.join(child.name))?;
            }
            return Ok(());
        }
        let mut src = self.lower.open(p, OpenMode::Read)?;
        let mut bytes = Vec::with_capacity(e.size as usize);
        let mut buf = [0u8; 8192];
        loop {
            match src.read(&mut buf)? {
                0 => break,
                n => bytes.extend_from_slice(&buf[..n]),
            }
        }
        let mut dst = self.upper.create(p, Attr(e.attr.0 & !Attr::READ_ONLY.0))?;
        dst.write(&bytes)?;
        dst.set_mtime(e.mtime)?;
        drop(dst);
        if e.attr.contains(Attr::READ_ONLY) {
            self.upper.set_attr(p, e.attr)?;
        }
        Ok(())
    }

    /// Merged listing of directory `dir`.
    fn merged(&mut self, dir: &DosPath) -> Result<Vec<DirEntry>, DosError> {
        let up = match self.upper.stat(dir) {
            _ if dir.is_root() => self.upper.read_dir(dir).ok(),
            Ok(e) if e.is_dir() => self.upper.read_dir(dir).ok(),
            _ => None,
        };
        let low = if self.whited(dir) {
            None
        } else {
            self.lower.read_dir(dir).ok()
        };
        if up.is_none() && low.is_none() {
            return Err(DosError::PathNotFound);
        }
        let mut out = BTreeMap::new();
        for e in low.into_iter().flatten() {
            if !self.whiteouts.contains(&dir.join(e.name)) {
                out.insert(e.name, e);
            }
        }
        for e in up.into_iter().flatten() {
            out.insert(e.name, e);
        }
        Ok(out.into_values().collect())
    }

    /// Hide the lower `p` (if any) after it was removed or moved.
    fn whiteout(&mut self, p: &DosPath) {
        if !self.whited(p) && self.lower.stat(p).is_ok() {
            self.whiteouts.insert(p.clone());
        }
    }
}

impl<L: Drive, U: Drive> Drive for OverlayDrive<L, U> {
    fn info(&self) -> DriveInfo {
        let (lo, up) = (self.lower.info(), self.upper.info());
        DriveInfo {
            label: lo.label,
            read_only: up.read_only,
            total_bytes: lo.total_bytes + up.total_bytes,
            free_bytes: up.free_bytes,
        }
    }

    fn open(&mut self, p: &DosPath, mode: OpenMode) -> Result<Box<dyn DosFile>, DosError> {
        if self.in_upper(p).is_some() {
            return self.upper.open(p, mode);
        }
        if !mode.can_write() {
            if self.in_lower(p).is_none() {
                return Err(self.missing(p));
            }
            return self.lower.open(p, mode);
        }
        match self.lookup(p) {
            Some(e) if e.is_dir() => return Err(DosError::AccessDenied),
            Some(e) if e.attr.contains(Attr::READ_ONLY) => return Err(DosError::AccessDenied),
            Some(_) => {}
            None => return Err(self.missing(p)),
        }
        self.copy_up(p)?;
        self.upper.open(p, mode)
    }

    fn create(&mut self, p: &DosPath, attr: Attr) -> Result<Box<dyn DosFile>, DosError> {
        let parent = p.parent().ok_or(DosError::AccessDenied)?;
        match self.lookup(p) {
            Some(e) if e.is_dir() || e.attr.contains(Attr::READ_ONLY) => {
                return Err(DosError::AccessDenied);
            }
            _ => {}
        }
        self.upper_dirs(&parent)?;
        let f = self.upper.create(p, attr)?;
        self.whiteouts.remove(p);
        Ok(f)
    }

    fn stat(&mut self, p: &DosPath) -> Result<DirEntry, DosError> {
        match self.lookup(p) {
            Some(e) => Ok(e),
            None => Err(self.missing(p)),
        }
    }

    fn read_dir(&mut self, dir: &DosPath) -> Result<Vec<DirEntry>, DosError> {
        self.merged(dir)
    }

    fn remove(&mut self, p: &DosPath) -> Result<(), DosError> {
        match self.lookup(p) {
            None => return Err(self.missing(p)),
            Some(e) if e.is_dir() || e.attr.contains(Attr::READ_ONLY) => {
                return Err(DosError::AccessDenied);
            }
            Some(_) => {}
        }
        if self.in_upper(p).is_some() {
            self.upper.remove(p)?;
        }
        self.whiteout(p);
        Ok(())
    }

    fn rename(&mut self, from: &DosPath, to: &DosPath) -> Result<(), DosError> {
        let e = match self.lookup(from) {
            Some(e) => e,
            None => return Err(self.missing(from)),
        };
        let to_parent = to.parent().ok_or(DosError::AccessDenied)?;
        if self.lookup(to).is_some() || (e.is_dir() && to.starts_with(from)) {
            return Err(DosError::AccessDenied);
        }
        if !self.is_dir(&to_parent) {
            return Err(DosError::PathNotFound);
        }
        self.copy_up(from)?;
        self.upper_dirs(&to_parent)?;
        // A whiteout under `to` would hide the moved entry's children.
        self.whiteouts.retain(|w| !w.starts_with(to));
        self.upper.rename(from, to)?;
        self.whiteout(from);
        Ok(())
    }

    fn mkdir(&mut self, p: &DosPath) -> Result<(), DosError> {
        let parent = p.parent().ok_or(DosError::AccessDenied)?;
        if self.lookup(p).is_some() {
            return Err(DosError::AccessDenied);
        }
        self.upper_dirs(&parent)?;
        self.upper.mkdir(p)?;
        // A re-created lower directory starts empty: hide its old children.
        if self.whiteouts.remove(p) {
            if let Ok(children) = self.lower.read_dir(p) {
                for c in children {
                    self.whiteouts.insert(p.join(c.name));
                }
            }
        }
        Ok(())
    }

    fn rmdir(&mut self, p: &DosPath) -> Result<(), DosError> {
        match self.lookup(p) {
            Some(e) if e.is_dir() => {}
            Some(_) | None if p.is_root() => return Err(DosError::AccessDenied),
            _ => return Err(DosError::PathNotFound),
        }
        if !self.merged(p)?.is_empty() {
            return Err(DosError::AccessDenied);
        }
        if self.in_upper(p).is_some() {
            self.upper.rmdir(p)?;
        }
        self.whiteout(p);
        Ok(())
    }

    fn set_attr(&mut self, p: &DosPath, attr: Attr) -> Result<(), DosError> {
        if self.lookup(p).is_none() {
            return Err(self.missing(p));
        }
        self.copy_up(p)?;
        self.upper.set_attr(p, attr)
    }
}
