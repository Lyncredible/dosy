//! Test helpers.

use std::fs;
use std::path::PathBuf;

/// Temporary directory removed on drop.
pub struct Tmp(pub PathBuf);

impl Tmp {
    pub fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("dosy-vfs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    pub fn put(&self, rel: &str, bytes: &[u8]) {
        let p = self.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
