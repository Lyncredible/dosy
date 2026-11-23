//! Read-only drive backed by a `.zip` archive.
//!
//! The central directory is indexed at mount into a tree (directories that
//! only appear as path prefixes are created implicitly), each directory's
//! names mapped to 8.3 with [`shortname::map_dir`]. Files are inflated into
//! memory when opened. Names use the archive's UTF-8 flag or CP437, as
//! decoded by the `zip` crate.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use zip::ZipArchive;

use crate::drive::dos_seek;
use crate::{
    Attr, DirEntry, DosDateTime, DosError, DosFile, DosName, DosPath, Drive, DriveInfo, OpenMode,
    SeekFrom, shortname,
};

/// Host-side tree while indexing (long names).
#[derive(Default)]
struct LongDir {
    dirs: BTreeMap<String, LongDir>,
    files: BTreeMap<String, FileInfo>,
    mtime: DosDateTime,
}

#[derive(Clone, Copy)]
struct FileInfo {
    index: usize,
    size: u32,
    mtime: DosDateTime,
}

enum Node {
    Dir(Dir),
    File(FileInfo),
}

struct Dir {
    /// In mapped (sorted host-name) order.
    entries: Vec<(DosName, Node)>,
    mtime: DosDateTime,
}

impl Dir {
    fn get(&self, n: &DosName) -> Option<&Node> {
        self.entries.iter().find(|(k, _)| k == n).map(|(_, v)| v)
    }
}

impl LongDir {
    /// Directory for `parts`, creating it implicitly.
    fn dir_mut(&mut self, parts: &[&str]) -> &mut LongDir {
        let mut d = self;
        for p in parts {
            d = d.dirs.entry(p.to_string()).or_default();
        }
        d
    }

    fn into_dir(self) -> Dir {
        let LongDir {
            mut dirs,
            mut files,
            mtime,
        } = self;
        // A file and a directory with the same name: the directory wins.
        files.retain(|k, _| !dirs.contains_key(k));
        let names: Vec<&String> = dirs.keys().chain(files.keys()).collect();
        let entries = shortname::map_dir(&names)
            .into_iter()
            .map(|(short, long)| {
                let node = match dirs.remove(&long) {
                    Some(d) => Node::Dir(d.into_dir()),
                    None => Node::File(files[&long]),
                };
                (short, node)
            })
            .collect();
        Dir { entries, mtime }
    }
}

fn zip_err(e: zip::result::ZipError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// A zip archive as a read-only DOS drive.
pub struct ZipDrive {
    archive: ZipArchive<File>,
    root: Dir,
    label: String,
    total: u64,
}

impl ZipDrive {
    pub fn open(path: impl AsRef<Path>, label: &str) -> io::Result<Self> {
        let mut archive = ZipArchive::new(File::open(path)?).map_err(zip_err)?;
        let mut tree = LongDir::default();
        let mut total = 0u64;
        for i in 0..archive.len() {
            let f = archive.by_index_raw(i).map_err(zip_err)?;
            let name = f.name().replace('\\', "/");
            let parts: Vec<&str> = name
                .split('/')
                .filter(|p| !p.is_empty() && *p != ".")
                .collect();
            // Skip entries that try to climb out of the archive.
            let Some((last, parents)) = parts.split_last() else {
                continue;
            };
            if parts.contains(&"..") {
                continue;
            }
            let mtime = f
                .last_modified()
                .map(|t| DosDateTime::from_dos(t.datepart(), t.timepart()))
                .unwrap_or_default();
            let parent = tree.dir_mut(parents);
            if f.is_dir() {
                parent.dirs.entry(last.to_string()).or_default().mtime = mtime;
            } else {
                let size = f.size().min(u32::MAX as u64) as u32;
                total += size as u64;
                parent.files.insert(
                    last.to_string(),
                    FileInfo {
                        index: i,
                        size,
                        mtime,
                    },
                );
            }
        }
        Ok(Self {
            archive,
            root: tree.into_dir(),
            label: label.to_string(),
            total,
        })
    }

    fn dir(&self, p: &DosPath) -> Result<&Dir, DosError> {
        let mut d = &self.root;
        for c in p.components() {
            d = match d.get(c) {
                Some(Node::Dir(x)) => x,
                _ => return Err(DosError::PathNotFound),
            };
        }
        Ok(d)
    }

    fn node(&self, p: &DosPath) -> Result<&Node, DosError> {
        let (parent, name) = match (p.parent(), p.file_name()) {
            (Some(parent), Some(name)) => (parent, name),
            _ => return Err(DosError::FileNotFound),
        };
        self.dir(&parent)?.get(name).ok_or(DosError::FileNotFound)
    }
}

fn entry(name: DosName, node: &Node) -> DirEntry {
    match node {
        Node::Dir(d) => DirEntry {
            name,
            attr: Attr::DIRECTORY,
            size: 0,
            mtime: d.mtime,
        },
        Node::File(f) => DirEntry {
            name,
            // Not marked read-only: an overlay may make it writable.
            attr: Attr::ARCHIVE,
            size: f.size,
            mtime: f.mtime,
        },
    }
}

impl Drive for ZipDrive {
    fn info(&self) -> DriveInfo {
        DriveInfo {
            label: self.label.clone(),
            read_only: true,
            total_bytes: self.total,
            free_bytes: 0,
        }
    }

    fn open(&mut self, p: &DosPath, mode: OpenMode) -> Result<Box<dyn DosFile>, DosError> {
        let info = match self.node(p)? {
            Node::File(f) => *f,
            Node::Dir(_) => return Err(DosError::AccessDenied),
        };
        if mode.can_write() {
            return Err(DosError::AccessDenied);
        }
        let mut f = self
            .archive
            .by_index(info.index)
            .map_err(|_| DosError::AccessDenied)?;
        let mut data = Vec::with_capacity(info.size as usize);
        f.read_to_end(&mut data)
            .map_err(|_| DosError::AccessDenied)?;
        Ok(Box::new(BufFile {
            data,
            pos: 0,
            mtime: info.mtime,
        }))
    }

    fn create(&mut self, _: &DosPath, _: Attr) -> Result<Box<dyn DosFile>, DosError> {
        Err(DosError::AccessDenied)
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

    fn remove(&mut self, _: &DosPath) -> Result<(), DosError> {
        Err(DosError::AccessDenied)
    }
    fn rename(&mut self, _: &DosPath, _: &DosPath) -> Result<(), DosError> {
        Err(DosError::AccessDenied)
    }
    fn mkdir(&mut self, _: &DosPath) -> Result<(), DosError> {
        Err(DosError::AccessDenied)
    }
    fn rmdir(&mut self, _: &DosPath) -> Result<(), DosError> {
        Err(DosError::AccessDenied)
    }
    fn set_attr(&mut self, _: &DosPath, _: Attr) -> Result<(), DosError> {
        Err(DosError::AccessDenied)
    }
}

/// A read-only in-memory file (an inflated zip entry).
struct BufFile {
    data: Vec<u8>,
    pos: u64,
    mtime: DosDateTime,
}

impl DosFile for BufFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, DosError> {
        let start = (self.pos as usize).min(self.data.len());
        let n = buf.len().min(self.data.len() - start);
        buf[..n].copy_from_slice(&self.data[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
    fn write(&mut self, _: &[u8]) -> Result<usize, DosError> {
        Err(DosError::AccessDenied)
    }
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, DosError> {
        self.pos = dos_seek(self.pos, self.data.len() as u64, pos);
        Ok(self.pos)
    }
    fn set_len(&mut self, _: u64) -> Result<(), DosError> {
        Err(DosError::AccessDenied)
    }
    fn mtime(&self) -> DosDateTime {
        self.mtime
    }
    fn set_mtime(&mut self, _: DosDateTime) -> Result<(), DosError> {
        Err(DosError::AccessDenied)
    }
}
