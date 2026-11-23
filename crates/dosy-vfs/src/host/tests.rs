use super::*;

/// Temporary directory removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("dosy-vfs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn put(&self, rel: &str, bytes: &[u8]) {
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

fn p(s: &str) -> DosPath {
    DosPath::parse(s).unwrap()
}

fn drive(t: &Tmp) -> HostDrive {
    HostDrive::new(&t.0, &MountOptions::default()).unwrap()
}

fn names(d: &mut HostDrive, dir: &str) -> Vec<String> {
    d.read_dir(&p(dir))
        .unwrap()
        .iter()
        .map(|e| e.name.to_string())
        .collect()
}

fn read_all(d: &mut HostDrive, path: &str) -> Vec<u8> {
    let mut f = d.open(&p(path), OpenMode::Read).unwrap();
    let mut buf = vec![0; 256];
    let n = f.read(&mut buf).unwrap();
    buf.truncate(n);
    buf
}

#[test]
fn maps_long_mixed_case_and_duplicate_names() {
    let t = Tmp::new("map");
    t.put("Long File Name.text", b"1");
    t.put("LongFileNameB.txt", b"2");
    t.put("Mixed.Txt", b"3");
    t.put("Sub Dir/inner file.dat", b"4");
    let mut d = drive(&t);
    let mut got = names(&mut d, "\\");
    got.sort();
    assert_eq!(
        got,
        ["LONGFI~1.TEX", "LONGFI~1.TXT", "MIXED.TXT", "SUBDIR~1"]
    );
    // Case-insensitive lookup through DOS names, nested directories.
    assert_eq!(read_all(&mut d, "longfi~1.tex"), b"1");
    assert_eq!(read_all(&mut d, "mixed.txt"), b"3");
    assert_eq!(read_all(&mut d, "subdir~1\\innerf~1.dat"), b"4");
    let e = d.stat(&p("SUBDIR~1")).unwrap();
    assert!(e.is_dir());
    assert_eq!(d.stat(&p("NOPE.TXT")), Err(DosError::FileNotFound));
    assert_eq!(
        d.stat(&p("NODIR\\X.TXT")).map(|_| ()),
        Err(DosError::PathNotFound)
    );
}

#[test]
fn round_trip_writes() {
    let t = Tmp::new("rw");
    let mut d = drive(&t);
    let mut f = d.create(&p("NEW.TXT"), Attr::NONE).unwrap();
    f.write(b"hello world").unwrap();
    assert_eq!(f.seek(SeekFrom::Start(6)).unwrap(), 6);
    f.write(b"DOS!!").unwrap();
    f.set_len(9).unwrap();
    let when = DosDateTime::from_dos(0x2A21, 0x6000); // 2001-01-01 12:00
    f.set_mtime(when).unwrap();
    // Seeking before the start wraps like DOS; read then sees EOF.
    assert_eq!(f.seek(SeekFrom::Current(-100)).unwrap(), 0xFFFF_FFFF - 88);
    let mut buf = [0; 4];
    assert_eq!(f.read(&mut buf).unwrap(), 0);
    drop(f);
    assert_eq!(fs::read(t.0.join("NEW.TXT")).unwrap(), b"hello DOS");
    assert_eq!(d.stat(&p("new.txt")).unwrap().mtime, when);
    assert_eq!(d.stat(&p("new.txt")).unwrap().size, 9);

    d.mkdir(&p("DIR")).unwrap();
    assert_eq!(d.mkdir(&p("DIR")), Err(DosError::AccessDenied));
    d.rename(&p("NEW.TXT"), &p("DIR\\MOVED.TXT")).unwrap();
    assert_eq!(read_all(&mut d, "DIR\\MOVED.TXT"), b"hello DOS");
    assert_eq!(d.rmdir(&p("DIR")), Err(DosError::AccessDenied), "not empty");
    d.remove(&p("DIR\\MOVED.TXT")).unwrap();
    d.rmdir(&p("DIR")).unwrap();
    assert!(names(&mut d, "\\").is_empty());

    // Creating through a short alias reuses the long host file.
    t.put("A long name.txt", b"old");
    d.create(&p("ALONGN~1.TXT"), Attr::NONE)
        .unwrap()
        .write(b"new")
        .unwrap();
    assert_eq!(fs::read(t.0.join("A long name.txt")).unwrap(), b"new");
}

#[test]
fn read_only_enforcement() {
    let t = Tmp::new("ro");
    t.put("FILE.TXT", b"data");
    let opts = MountOptions {
        read_only: true,
        label: "RO".into(),
    };
    let mut d = HostDrive::new(&t.0, &opts).unwrap();
    assert_eq!(read_all(&mut d, "FILE.TXT"), b"data");
    let denied = Err(DosError::AccessDenied);
    assert_eq!(d.open(&p("FILE.TXT"), OpenMode::Write).map(|_| ()), denied);
    assert_eq!(d.create(&p("X.TXT"), Attr::NONE).map(|_| ()), denied);
    assert_eq!(d.remove(&p("FILE.TXT")), denied);
    assert_eq!(d.mkdir(&p("D")), denied);
    assert_eq!(d.rename(&p("FILE.TXT"), &p("Y.TXT")), denied);
    assert!(d.info().read_only);
    assert_eq!(fs::read(t.0.join("FILE.TXT")).unwrap(), b"data");

    // Read-only attribute on a writable drive.
    let mut d = drive(&t);
    d.set_attr(&p("FILE.TXT"), Attr::READ_ONLY).unwrap();
    assert!(
        d.stat(&p("FILE.TXT"))
            .unwrap()
            .attr
            .contains(Attr::READ_ONLY)
    );
    assert_eq!(
        d.open(&p("FILE.TXT"), OpenMode::ReadWrite).map(|_| ()),
        denied
    );
    assert_eq!(d.remove(&p("FILE.TXT")), denied);
    d.set_attr(&p("FILE.TXT"), Attr::NONE).unwrap();
    d.remove(&p("FILE.TXT")).unwrap();
}

#[test]
fn root_escape_is_rejected() {
    // DOS paths cannot climb above the root at all.
    assert_eq!(DosPath::parse("..\\X"), Err(DosError::PathNotFound));
    let t = Tmp::new("escape");
    let outside = Tmp::new("outside");
    outside.put("SECRET.TXT", b"x");
    t.put("IN.TXT", b"y");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside.0, t.0.join("LINK")).unwrap();
        std::os::unix::fs::symlink(outside.0.join("SECRET.TXT"), t.0.join("S.TXT")).unwrap();
        let mut d = drive(&t);
        assert_eq!(names(&mut d, "\\"), ["IN.TXT"], "links out are hidden");
        assert!(d.open(&p("LINK\\SECRET.TXT"), OpenMode::Read).is_err());
        assert!(d.open(&p("S.TXT"), OpenMode::Read).is_err());
        assert!(d.read_dir(&p("LINK")).is_err());
    }
    let mut d = drive(&t);
    assert_eq!(read_all(&mut d, "IN.TXT"), b"y");
}

#[test]
fn single_file_mount() {
    let t = Tmp::new("single");
    t.put("game data.bin", b"payload");
    t.put("OTHER.TXT", b"hidden");
    let file = t.0.join("game data.bin");
    let mut d = open_mount(&file, &MountOptions::default()).unwrap();
    let list: Vec<String> = d
        .read_dir(&DosPath::root())
        .unwrap()
        .iter()
        .map(|e| e.name.to_string())
        .collect();
    assert_eq!(list, ["GAMEDA~1.BIN"]);
    assert!(d.open(&p("OTHER.TXT"), OpenMode::Read).is_err());
    assert_eq!(
        d.create(&p("NEW.TXT"), Attr::NONE).map(|_| ()),
        Err(DosError::AccessDenied)
    );
    assert_eq!(d.mkdir(&p("D")), Err(DosError::AccessDenied));
    let mut f = d.open(&p("GAMEDA~1.BIN"), OpenMode::ReadWrite).unwrap();
    f.write(b"P").unwrap();
    drop(f);
    assert_eq!(fs::read(&file).unwrap(), b"Payload");

    assert!(open_mount(&t.0, &MountOptions::default()).is_ok());
    assert!(open_mount(&t.0.join("missing"), &MountOptions::default()).is_err());
}

#[test]
fn dos_time_conversion() {
    let t = DosDateTime::from_dos(0x5B5A, 0x8C64); // 2025-10-26 17:35:08
    assert_eq!(to_dos_time(from_dos_time(t)), t);
    assert_eq!(to_dos_time(UNIX_EPOCH), DosDateTime::default());
    let leap = DosDateTime {
        year: 2024,
        month: 2,
        day: 29,
        hour: 23,
        minute: 59,
        second: 58,
    };
    assert_eq!(to_dos_time(from_dos_time(leap)), leap);
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(
        civil_from_days(days_from_civil(2107, 12, 31)),
        (2107, 12, 31)
    );
}
