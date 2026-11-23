//! In-memory drive, mainly for tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::{
    Attr, DirEntry, DosDateTime, DosError, DosFile, DosName, DosPath, Drive, DriveInfo, OpenMode,
    SeekFrom,
};

/// Largest file size (DOS uses 32-bit offsets).
const MAX_FILE: u64 = u32::MAX as u64;
/// Reported drive size.
const TOTAL_BYTES: u64 = 64 << 20;
/// Attributes a file may carry.
const FILE_ATTRS: u8 = 0x27;

struct FileData {
    bytes: Vec<u8>,
    attr: Attr,
    mtime: DosDateTime,
}

type Shared = Arc<Mutex<FileData>>;

fn lock(f: &Shared) -> MutexGuard<'_, FileData> {
    f.lock().unwrap_or_else(|e| e.into_inner())
}

enum Node {
    File(Shared),
    Dir(Dir),
}

#[derive(Default)]
struct Dir {
    entries: BTreeMap<DosName, Node>,
    mtime: DosDateTime,
}

impl Dir {
    fn size(&self) -> u64 {
        self.entries
            .values()
            .map(|n| match n {
                Node::File(f) => lock(f).bytes.len() as u64,
                Node::Dir(d) => d.size(),
            })
            .sum()
    }
}

fn entry(name: DosName, node: &Node) -> DirEntry {
    match node {
        Node::File(f) => {
            let f = lock(f);
            DirEntry {
                name,
                attr: f.attr,
                size: f.bytes.len() as u32,
                mtime: f.mtime,
            }
        }
        Node::Dir(d) => DirEntry {
            name,
            attr: Attr::DIRECTORY,
            size: 0,
            mtime: d.mtime,
        },
    }
}

/// A drive whose tree lives in memory. Entries list in name order. Open
/// handles share file contents with the tree (and each other).
#[derive(Default)]
pub struct MemDrive {
    root: Dir,
    label: String,
    read_only: bool,
    /// Timestamp given to new files and directories.
    pub now: DosDateTime,
}

impl MemDrive {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_label(label: &str) -> Self {
        Self {
            label: label.to_string(),
            ..Self::default()
        }
    }

    pub fn set_read_only(&mut self, ro: bool) {
        self.read_only = ro;
    }

    /// Test helper: create `path` (and missing parents) with `bytes`, even
    /// on a read-only drive. Panics on an invalid path.
    pub fn add_file(&mut self, path: &str, bytes: &[u8]) {
        let p = DosPath::parse(path).expect("valid path");
        let (name, parents) = p.components().split_last().expect("not the root");
        let now = self.now;
        let mut d = &mut self.root;
        for c in parents {
            let node = d.entries.entry(*c).or_insert_with(|| {
                Node::Dir(Dir {
                    entries: BTreeMap::new(),
                    mtime: now,
                })
            });
            d = match node {
                Node::Dir(x) => x,
                Node::File(_) => panic!("{c} is a file"),
            };
        }
        d.entries.insert(
            *name,
            Node::File(new_file(bytes.to_vec(), Attr::ARCHIVE, now)),
        );
    }

    /// Test helper: contents of the file at `path`.
    pub fn file_bytes(&self, path: &str) -> Option<Vec<u8>> {
        let p = DosPath::parse(path).ok()?;
        match self.node(&p).ok()? {
            Node::File(f) => Some(lock(f).bytes.clone()),
            Node::Dir(_) => None,
        }
    }

    fn writable(&self) -> Result<(), DosError> {
        if self.read_only {
            Err(DosError::AccessDenied)
        } else {
            Ok(())
        }
    }

    fn dir(&self, p: &DosPath) -> Result<&Dir, DosError> {
        let mut d = &self.root;
        for c in p.components() {
            d = match d.entries.get(c) {
                Some(Node::Dir(x)) => x,
                _ => return Err(DosError::PathNotFound),
            };
        }
        Ok(d)
    }

    fn dir_mut(&mut self, p: &DosPath) -> Result<&mut Dir, DosError> {
        let mut d = &mut self.root;
        for c in p.components() {
            d = match d.entries.get_mut(c) {
                Some(Node::Dir(x)) => x,
                _ => return Err(DosError::PathNotFound),
            };
        }
        Ok(d)
    }

    /// Parent directory and final name of a non-root path.
    fn split(p: &DosPath) -> Result<(DosPath, DosName), DosError> {
        match (p.parent(), p.file_name()) {
            (Some(parent), Some(name)) => Ok((parent, *name)),
            _ => Err(DosError::AccessDenied),
        }
    }

    fn node(&self, p: &DosPath) -> Result<&Node, DosError> {
        let (parent, name) = Self::split(p)?;
        self.dir(&parent)?
            .entries
            .get(&name)
            .ok_or(DosError::FileNotFound)
    }
}

fn new_file(bytes: Vec<u8>, attr: Attr, mtime: DosDateTime) -> Shared {
    Arc::new(Mutex::new(FileData {
        bytes,
        attr: Attr(attr.0 & FILE_ATTRS),
        mtime,
    }))
}

impl Drive for MemDrive {
    fn info(&self) -> DriveInfo {
        DriveInfo {
            label: self.label.clone(),
            read_only: self.read_only,
            total_bytes: TOTAL_BYTES,
            free_bytes: TOTAL_BYTES.saturating_sub(self.root.size()),
        }
    }

    fn open(&mut self, p: &DosPath, mode: OpenMode) -> Result<Box<dyn DosFile>, DosError> {
        let Node::File(f) = self.node(p)? else {
            return Err(DosError::AccessDenied);
        };
        if mode.can_write() && (self.read_only || lock(f).attr.contains(Attr::READ_ONLY)) {
            return Err(DosError::AccessDenied);
        }
        Ok(Box::new(MemFile {
            data: f.clone(),
            pos: 0,
            mode,
        }))
    }

    fn create(&mut self, p: &DosPath, attr: Attr) -> Result<Box<dyn DosFile>, DosError> {
        self.writable()?;
        let (parent, name) = Self::split(p)?;
        let now = self.now;
        let dir = self.dir_mut(&parent)?;
        let data = match dir.entries.get(&name) {
            Some(Node::Dir(_)) => return Err(DosError::AccessDenied),
            Some(Node::File(f)) => {
                let mut fd = lock(f);
                if fd.attr.contains(Attr::READ_ONLY) {
                    return Err(DosError::AccessDenied);
                }
                fd.bytes.clear();
                fd.attr = Attr(attr.0 & FILE_ATTRS);
                fd.mtime = now;
                drop(fd);
                f.clone()
            }
            None => {
                let f = new_file(Vec::new(), attr, now);
                dir.entries.insert(name, Node::File(f.clone()));
                f
            }
        };
        Ok(Box::new(MemFile {
            data,
            pos: 0,
            mode: OpenMode::ReadWrite,
        }))
    }

    fn stat(&mut self, p: &DosPath) -> Result<DirEntry, DosError> {
        let name = *p.file_name().ok_or(DosError::FileNotFound)?;
        Ok(entry(name, self.node(p)?))
    }

    fn read_dir(&mut self, dir: &DosPath) -> Result<Vec<DirEntry>, DosError> {
        Ok(self
            .dir(dir)?
            .entries
            .iter()
            .map(|(n, node)| entry(*n, node))
            .collect())
    }

    fn remove(&mut self, p: &DosPath) -> Result<(), DosError> {
        self.writable()?;
        match self.node(p)? {
            Node::Dir(_) => return Err(DosError::AccessDenied),
            Node::File(f) if lock(f).attr.contains(Attr::READ_ONLY) => {
                return Err(DosError::AccessDenied);
            }
            Node::File(_) => {}
        }
        let (parent, name) = Self::split(p)?;
        self.dir_mut(&parent)?.entries.remove(&name);
        Ok(())
    }

    fn rename(&mut self, from: &DosPath, to: &DosPath) -> Result<(), DosError> {
        self.writable()?;
        let is_dir = matches!(self.node(from)?, Node::Dir(_));
        let (to_parent, to_name) = Self::split(to)?;
        if self.dir(&to_parent)?.entries.contains_key(&to_name) || (is_dir && to.starts_with(from))
        {
            return Err(DosError::AccessDenied);
        }
        let (from_parent, from_name) = Self::split(from)?;
        let node = self
            .dir_mut(&from_parent)?
            .entries
            .remove(&from_name)
            .ok_or(DosError::FileNotFound)?;
        self.dir_mut(&to_parent)?.entries.insert(to_name, node);
        Ok(())
    }

    fn mkdir(&mut self, p: &DosPath) -> Result<(), DosError> {
        self.writable()?;
        let (parent, name) = Self::split(p)?;
        let now = self.now;
        let dir = self.dir_mut(&parent)?;
        if dir.entries.contains_key(&name) {
            return Err(DosError::AccessDenied);
        }
        dir.entries.insert(
            name,
            Node::Dir(Dir {
                entries: BTreeMap::new(),
                mtime: now,
            }),
        );
        Ok(())
    }

    fn rmdir(&mut self, p: &DosPath) -> Result<(), DosError> {
        self.writable()?;
        match self.node(p) {
            Ok(Node::Dir(d)) if d.entries.is_empty() => {}
            Ok(Node::Dir(_)) => return Err(DosError::AccessDenied),
            Ok(Node::File(_)) | Err(DosError::FileNotFound) => {
                return Err(DosError::PathNotFound);
            }
            Err(e) => return Err(e),
        }
        let (parent, name) = Self::split(p)?;
        self.dir_mut(&parent)?.entries.remove(&name);
        Ok(())
    }

    fn set_attr(&mut self, p: &DosPath, attr: Attr) -> Result<(), DosError> {
        self.writable()?;
        if let Node::File(f) = self.node(p)? {
            lock(f).attr = Attr(attr.0 & FILE_ATTRS);
        }
        Ok(())
    }
}

struct MemFile {
    data: Shared,
    pos: u64,
    mode: OpenMode,
}

impl DosFile for MemFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, DosError> {
        if !self.mode.can_read() {
            return Err(DosError::AccessDenied);
        }
        let f = lock(&self.data);
        let start = (self.pos as usize).min(f.bytes.len());
        let n = buf.len().min(f.bytes.len() - start);
        buf[..n].copy_from_slice(&f.bytes[start..start + n]);
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
        let end = self.pos + buf.len() as u64;
        if end > MAX_FILE {
            return Err(DosError::InsufficientMemory);
        }
        let mut f = lock(&self.data);
        let (start, end) = (self.pos as usize, end as usize);
        if f.bytes.len() < end {
            f.bytes.resize(end, 0);
        }
        f.bytes[start..end].copy_from_slice(buf);
        f.attr |= Attr::ARCHIVE;
        self.pos = end as u64;
        Ok(buf.len())
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, DosError> {
        let len = lock(&self.data).bytes.len() as u64;
        self.pos = crate::drive::dos_seek(self.pos, len, pos);
        Ok(self.pos)
    }

    fn set_len(&mut self, len: u64) -> Result<(), DosError> {
        if !self.mode.can_write() {
            return Err(DosError::AccessDenied);
        }
        if len > MAX_FILE {
            return Err(DosError::InsufficientMemory);
        }
        lock(&self.data).bytes.resize(len as usize, 0);
        Ok(())
    }

    fn mtime(&self) -> DosDateTime {
        lock(&self.data).mtime
    }

    fn set_mtime(&mut self, t: DosDateTime) -> Result<(), DosError> {
        lock(&self.data).mtime = t;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> DosPath {
        DosPath::parse(s).unwrap()
    }

    fn names(d: &mut MemDrive, dir: &str) -> Vec<String> {
        d.read_dir(&p(dir))
            .unwrap()
            .iter()
            .map(|e| e.name.to_string())
            .collect()
    }

    fn read_all(f: &mut dyn DosFile) -> Vec<u8> {
        f.seek(SeekFrom::Start(0)).unwrap();
        let mut out = vec![0; 64];
        let n = f.read(&mut out).unwrap();
        out.truncate(n);
        out
    }

    #[test]
    fn crud() {
        let mut d = MemDrive::with_label("TEST");
        d.mkdir(&p("\\APPS")).unwrap();
        let mut f = d.create(&p("\\apps\\a.txt"), Attr::NONE).unwrap();
        assert_eq!(f.write(b"hello").unwrap(), 5);
        assert_eq!(read_all(&mut *f), b"hello");
        drop(f);
        let e = d.stat(&p("APPS\\A.TXT")).unwrap();
        assert_eq!((e.size, e.attr), (5, Attr::ARCHIVE));
        assert!(d.stat(&p("APPS")).unwrap().is_dir());
        d.add_file("\\APPS\\B.EXE", b"MZ");
        assert_eq!(names(&mut d, "APPS"), ["A.TXT", "B.EXE"]);
        assert_eq!(names(&mut d, "\\"), ["APPS"]);

        // Opened handles share contents; create truncates.
        let mut r = d.open(&p("APPS\\A.TXT"), OpenMode::Read).unwrap();
        let mut c = d.create(&p("APPS\\A.TXT"), Attr::NONE).unwrap();
        c.write(b"x").unwrap();
        assert_eq!(read_all(&mut *r), b"x");
        assert_eq!(r.write(b"y"), Err(DosError::AccessDenied));

        d.remove(&p("APPS\\A.TXT")).unwrap();
        assert_eq!(d.stat(&p("APPS\\A.TXT")), Err(DosError::FileNotFound));
        assert_eq!(d.rmdir(&p("APPS")), Err(DosError::AccessDenied));
        d.remove(&p("APPS\\B.EXE")).unwrap();
        d.rmdir(&p("APPS")).unwrap();
        assert!(names(&mut d, "\\").is_empty());
        assert_eq!(d.info().label, "TEST");
        assert_eq!(d.info().free_bytes, d.info().total_bytes);
    }

    #[test]
    fn error_codes() {
        let mut d = MemDrive::new();
        d.add_file("DIR\\F.TXT", b"1");
        let missing = d.open(&p("NOPE.TXT"), OpenMode::Read).err();
        assert_eq!(missing, Some(DosError::FileNotFound));
        let no_dir = d.open(&p("NODIR\\F.TXT"), OpenMode::Read).err();
        assert_eq!(no_dir, Some(DosError::PathNotFound));
        let via_file = d.create(&p("DIR\\F.TXT\\G"), Attr::NONE).err();
        assert_eq!(via_file, Some(DosError::PathNotFound));
        let open_dir = d.open(&p("DIR"), OpenMode::Read).err();
        assert_eq!(open_dir, Some(DosError::AccessDenied));
        assert_eq!(d.read_dir(&p("DIR\\F.TXT")), Err(DosError::PathNotFound));
        assert_eq!(d.mkdir(&p("DIR")), Err(DosError::AccessDenied));
        assert_eq!(d.remove(&p("DIR")), Err(DosError::AccessDenied));
        assert_eq!(d.rmdir(&p("DIR\\F.TXT")), Err(DosError::PathNotFound));
        assert_eq!(d.rmdir(&p("\\")), Err(DosError::AccessDenied));

        // Read-only attribute.
        d.set_attr(&p("DIR\\F.TXT"), Attr::READ_ONLY).unwrap();
        let ro = d.open(&p("DIR\\F.TXT"), OpenMode::ReadWrite).err();
        assert_eq!(ro, Some(DosError::AccessDenied));
        assert!(d.open(&p("DIR\\F.TXT"), OpenMode::Read).is_ok());
        assert_eq!(d.remove(&p("DIR\\F.TXT")), Err(DosError::AccessDenied));
        let trunc = d.create(&p("DIR\\F.TXT"), Attr::NONE).err();
        assert_eq!(trunc, Some(DosError::AccessDenied));
    }

    #[test]
    fn read_only_drive_rejects_writes() {
        let mut d = MemDrive::new();
        d.add_file("A.TXT", b"abc");
        d.mkdir(&p("D")).unwrap();
        d.set_read_only(true);
        assert!(d.info().read_only);
        let denied = Some(DosError::AccessDenied);
        assert_eq!(d.open(&p("A.TXT"), OpenMode::Write).err(), denied);
        assert_eq!(d.create(&p("B.TXT"), Attr::NONE).err(), denied);
        assert_eq!(d.remove(&p("A.TXT")).err(), denied);
        assert_eq!(d.rename(&p("A.TXT"), &p("B.TXT")).err(), denied);
        assert_eq!(d.mkdir(&p("E")).err(), denied);
        assert_eq!(d.rmdir(&p("D")).err(), denied);
        assert_eq!(d.set_attr(&p("A.TXT"), Attr::NONE).err(), denied);
        let mut f = d.open(&p("A.TXT"), OpenMode::Read).unwrap();
        assert_eq!(read_all(&mut *f), b"abc");
        assert_eq!(f.set_len(0), Err(DosError::AccessDenied));
    }

    #[test]
    fn seek_past_eof_then_write_zero_fills() {
        let mut d = MemDrive::new();
        let mut f = d.create(&p("F.BIN"), Attr::NONE).unwrap();
        f.write(b"ab").unwrap();
        assert_eq!(f.seek(SeekFrom::Start(5)).unwrap(), 5);
        let mut buf = [0; 4];
        assert_eq!(f.read(&mut buf).unwrap(), 0);
        assert_eq!(d.stat(&p("F.BIN")).unwrap().size, 2);
        f.write(b"z").unwrap();
        assert_eq!(read_all(&mut *f), b"ab\0\0\0z");
        assert_eq!(f.seek(SeekFrom::End(-1)).unwrap(), 5);
        assert_eq!(f.seek(SeekFrom::Current(-2)).unwrap(), 3);
        // Before the start: wraps like DOS; read sees EOF, write fails.
        assert_eq!(f.seek(SeekFrom::Current(-4)), Ok(0xFFFF_FFFF));
        assert_eq!(f.read(&mut buf).unwrap(), 0);
        assert!(f.write(b"x").is_err());
        f.set_len(1).unwrap();
        assert_eq!(read_all(&mut *f), b"a");
        let t = DosDateTime::from_dos(0x1234, 0x5678);
        f.set_mtime(t).unwrap();
        assert_eq!(d.stat(&p("F.BIN")).unwrap().mtime, t);
    }

    #[test]
    fn rename_across_dirs() {
        let mut d = MemDrive::new();
        d.add_file("A\\X.TXT", b"x");
        d.add_file("B\\Y.TXT", b"y");
        d.rename(&p("A\\X.TXT"), &p("B\\Z.TXT")).unwrap();
        assert_eq!(d.file_bytes("B\\Z.TXT").unwrap(), b"x");
        assert!(names(&mut d, "A").is_empty());
        let exists = d.rename(&p("B\\Z.TXT"), &p("B\\Y.TXT")).err();
        assert_eq!(exists, Some(DosError::AccessDenied));
        let missing = d.rename(&p("A\\X.TXT"), &p("B\\W.TXT")).err();
        assert_eq!(missing, Some(DosError::FileNotFound));
        let no_dir = d.rename(&p("B\\Y.TXT"), &p("C\\Y.TXT")).err();
        assert_eq!(no_dir, Some(DosError::PathNotFound));
        // Directories move with their contents, but not into themselves.
        d.rename(&p("B"), &p("A\\B2")).unwrap();
        assert_eq!(names(&mut d, "A\\B2"), ["Y.TXT", "Z.TXT"]);
        let into_self = d.rename(&p("A"), &p("A\\B2\\A")).err();
        assert_eq!(into_self, Some(DosError::AccessDenied));
        assert_eq!(names(&mut d, "\\"), ["A"]);
    }

    #[test]
    fn drive_and_files_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<MemDrive>();
        assert_send::<Box<dyn DosFile>>();
        assert_send::<Box<dyn Drive>>();
    }
}
