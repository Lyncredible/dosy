//! ZipDrive and OverlayDrive tests; archives are built at runtime.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::testutil::Tmp;
use crate::*;

const BIG: usize = 20_000;

fn big() -> Vec<u8> {
    (0..BIG).map(|i| (i * 7 % 251) as u8).collect()
}

/// Zip with stored and deflated files, nested and implicit directories,
/// long names and an explicit empty directory.
fn make_zip(path: &Path) {
    let mut w = ZipWriter::new(File::create(path).unwrap());
    let t = DateTime::from_date_and_time(1994, 12, 10, 23, 59, 58).unwrap();
    let deflate = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(t);
    let stored = deflate.compression_method(CompressionMethod::Stored);
    w.add_directory("Empty Dir/", stored).unwrap();
    for (name, bytes, opts) in [
        ("README.TXT", b"read me".to_vec(), stored),
        ("apps/tool.exe", big(), deflate),
        ("apps/Long Config Name.cfg", b"cfg".to_vec(), deflate),
        ("apps/data/levels/level1.dat", b"L1".to_vec(), stored),
    ] {
        w.start_file(name, opts).unwrap();
        w.write_all(&bytes).unwrap();
    }
    w.finish().unwrap();
}

fn p(s: &str) -> DosPath {
    DosPath::parse(s).unwrap()
}

fn names(d: &mut dyn Drive, dir: &str) -> Vec<String> {
    d.read_dir(&p(dir))
        .unwrap()
        .iter()
        .map(|e| e.name.to_string())
        .collect()
}

fn read(d: &mut dyn Drive, path: &str) -> Vec<u8> {
    let mut f = d.open(&p(path), OpenMode::Read).unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match f.read(&mut buf).unwrap() {
            0 => return out,
            n => out.extend_from_slice(&buf[..n]),
        }
    }
}

fn write(d: &mut dyn Drive, path: &str, bytes: &[u8]) {
    let mut f = d.create(&p(path), Attr::NONE).unwrap();
    f.write(bytes).unwrap();
}

fn zip_drive(t: &Tmp) -> ZipDrive {
    let path = t.0.join("TEST.ZIP");
    make_zip(&path);
    ZipDrive::open(&path, "ZIPPED").unwrap()
}

#[test]
fn zip_listing_nested_dirs_long_names_and_content() {
    let t = Tmp::new("zip-list");
    let mut d = zip_drive(&t);
    let mut root = names(&mut d, "\\");
    root.sort();
    assert_eq!(root, ["APPS", "EMPTYD~1", "README.TXT"]);
    let mut apps = names(&mut d, "apps");
    apps.sort();
    assert_eq!(apps, ["DATA", "LONGCO~1.CFG", "TOOL.EXE"]);
    assert_eq!(names(&mut d, "apps\\data"), ["LEVELS"]);
    assert!(names(&mut d, "EMPTYD~1").is_empty());
    assert_eq!(d.read_dir(&p("NOPE")), Err(DosError::PathNotFound));

    assert_eq!(read(&mut d, "readme.txt"), b"read me");
    assert_eq!(read(&mut d, "APPS\\TOOL.EXE"), big());
    assert_eq!(read(&mut d, "APPS\\LONGCO~1.CFG"), b"cfg");
    assert_eq!(read(&mut d, "APPS\\DATA\\LEVELS\\LEVEL1.DAT"), b"L1");

    let e = d.stat(&p("APPS\\TOOL.EXE")).unwrap();
    assert_eq!(e.size, BIG as u32);
    assert_eq!(
        e.mtime.to_dos(),
        ((14 << 9) | (12 << 5) | 10, (23 << 11) | (59 << 5) | 29)
    );
    assert!(d.stat(&p("APPS\\DATA")).unwrap().is_dir());
    assert_eq!(d.info().label, "ZIPPED");
    assert!(d.info().read_only);

    // Seeks, including DOS-style wrap before the start.
    let mut f = d.open(&p("APPS\\TOOL.EXE"), OpenMode::Read).unwrap();
    assert_eq!(f.seek(SeekFrom::End(-2)).unwrap(), BIG as u64 - 2);
    let mut b = [0; 4];
    assert_eq!(f.read(&mut b).unwrap(), 2);
    assert_eq!(b[..2], big()[BIG - 2..]);
    assert_eq!(f.seek(SeekFrom::Start(0)).unwrap(), 0);
    assert_eq!(f.seek(SeekFrom::Current(-1)).unwrap(), 0xFFFF_FFFF);
    assert_eq!(f.read(&mut b).unwrap(), 0);
}

#[test]
fn zip_rejects_writes_without_overlay() {
    let t = Tmp::new("zip-ro");
    let path = t.0.join("TEST.ZIP");
    make_zip(&path);
    let before = fs::read(&path).unwrap();
    let mut d = open_mount(&path, &MountOptions::default()).unwrap();
    let denied = Err(DosError::AccessDenied);
    assert_eq!(
        d.open(&p("README.TXT"), OpenMode::ReadWrite).map(|_| ()),
        denied
    );
    assert_eq!(d.create(&p("NEW.TXT"), Attr::NONE).map(|_| ()), denied);
    assert_eq!(d.remove(&p("README.TXT")), denied);
    assert_eq!(d.rename(&p("README.TXT"), &p("X.TXT")), denied);
    assert_eq!(d.mkdir(&p("D")), denied);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn overlay_copy_up_delete_rename() {
    let t = Tmp::new("zip-overlay");
    let zip_path = t.0.join("TEST.ZIP");
    let mut d = OverlayDrive::new(zip_drive(&t), MemDrive::new());
    let before = fs::read(&zip_path).unwrap();

    // Copy-up on write keeps the rest of the file.
    let mut f = d.open(&p("README.TXT"), OpenMode::ReadWrite).unwrap();
    f.write(b"READ").unwrap();
    drop(f);
    assert_eq!(read(&mut d, "README.TXT"), b"READ me");
    let e = d.stat(&p("README.TXT")).unwrap();
    assert_eq!(e.size, 7);

    // New file in a lower-only directory.
    write(&mut d, "APPS\\DATA\\SAVE.DAT", b"save");
    let mut data = names(&mut d, "APPS\\DATA");
    data.sort();
    assert_eq!(data, ["LEVELS", "SAVE.DAT"]);
    assert_eq!(read(&mut d, "APPS\\DATA\\LEVELS\\LEVEL1.DAT"), b"L1");

    // Delete a lower file: whiteout.
    d.remove(&p("APPS\\TOOL.EXE")).unwrap();
    assert_eq!(d.stat(&p("APPS\\TOOL.EXE")), Err(DosError::FileNotFound));
    assert!(!names(&mut d, "APPS").contains(&"TOOL.EXE".to_string()));
    // ...and re-create it.
    write(&mut d, "APPS\\TOOL.EXE", b"new");
    assert_eq!(read(&mut d, "APPS\\TOOL.EXE"), b"new");

    // Rename a lower file and a lower directory tree.
    d.rename(&p("APPS\\LONGCO~1.CFG"), &p("SETUP.CFG")).unwrap();
    assert_eq!(read(&mut d, "SETUP.CFG"), b"cfg");
    assert_eq!(
        d.stat(&p("APPS\\LONGCO~1.CFG")),
        Err(DosError::FileNotFound)
    );
    d.rename(&p("APPS\\DATA"), &p("SAVES")).unwrap();
    assert_eq!(read(&mut d, "SAVES\\LEVELS\\LEVEL1.DAT"), b"L1");
    assert_eq!(read(&mut d, "SAVES\\SAVE.DAT"), b"save");
    assert_eq!(d.read_dir(&p("APPS\\DATA")), Err(DosError::PathNotFound));
    assert_eq!(
        d.rename(&p("README.TXT"), &p("SETUP.CFG")),
        Err(DosError::AccessDenied)
    );

    // Remove an emptied lower directory, then re-create it empty.
    d.remove(&p("SAVES\\LEVELS\\LEVEL1.DAT")).unwrap();
    d.rmdir(&p("SAVES\\LEVELS")).unwrap();
    assert_eq!(
        d.rmdir(&p("SAVES")),
        Err(DosError::AccessDenied),
        "not empty"
    );
    d.rmdir(&p("EMPTYD~1")).unwrap();
    assert!(d.stat(&p("EMPTYD~1")).is_err());
    d.mkdir(&p("EMPTYD~1")).unwrap();
    assert!(names(&mut d, "EMPTYD~1").is_empty());

    let mut root = names(&mut d, "\\");
    root.sort();
    assert_eq!(
        root,
        ["APPS", "EMPTYD~1", "README.TXT", "SAVES", "SETUP.CFG"]
    );
    assert_eq!(fs::read(&zip_path).unwrap(), before, "zip untouched");
}

#[test]
fn open_mount_zip_with_directory_overlay() {
    let t = Tmp::new("zip-mount");
    let zip_path = t.0.join("Archive.Zip");
    make_zip(&zip_path);
    let upper = t.0.join("saves");
    let opts = MountOptions {
        overlay: Overlay::Dir(upper.clone()),
        ..Default::default()
    };
    let mut d = open_mount(&zip_path, &opts).unwrap();
    assert_eq!(read(&mut *d, "APPS\\TOOL.EXE"), big());
    write(&mut *d, "APPS\\HISCORE.DAT", b"999");
    let mut f = d.open(&p("README.TXT"), OpenMode::Write).unwrap();
    f.write(b"X").unwrap();
    drop(f);
    drop(d);
    assert_eq!(
        fs::read(upper.join("APPS").join("HISCORE.DAT")).unwrap(),
        b"999"
    );
    assert_eq!(fs::read(upper.join("README.TXT")).unwrap(), b"Xead me");

    // A fresh mount sees the persisted writes over the zip.
    let mut d = open_mount(&zip_path, &opts).unwrap();
    assert_eq!(read(&mut *d, "APPS\\HISCORE.DAT"), b"999");
    assert_eq!(read(&mut *d, "README.TXT"), b"Xead me");

    let mem = MountOptions {
        overlay: Overlay::Memory,
        ..Default::default()
    };
    let mut d = open_mount(&zip_path, &mem).unwrap();
    write(&mut *d, "NEW.TXT", b"n");
    assert_eq!(read(&mut *d, "NEW.TXT"), b"n");
}
