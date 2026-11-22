//! 8.3 names, wildcard patterns and drive-relative paths.

use std::fmt;

use crate::DosError;

/// Characters DOS allows in file names besides `A–Z`, `0–9` and bytes >= 0x80.
const SPECIAL: &[u8] = b"!#$%&'()-@^_`{}~";

fn legal(c: u8) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || c >= 0x80 || SPECIAL.contains(&c)
}

/// Split `NAME.EXT` and upper-case it; `None` if a field is too long, the
/// base is empty, or there is more than one dot.
fn split(s: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let up = s.as_bytes().to_ascii_uppercase();
    let (base, ext) = match up.iter().position(|&c| c == b'.') {
        Some(i) => (up[..i].to_vec(), up[i + 1..].to_vec()),
        None => (up, Vec::new()),
    };
    (!base.is_empty() && !ext.contains(&b'.')).then_some((base, ext))
}

/// A validated 8.3 name, stored in FCB form: 8 + 3 bytes, space padded,
/// uppercase. Ordering is by the FCB bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DosName([u8; 11]);

impl DosName {
    /// Parse `NAME.EXT` (case-insensitive). `.` and `..` are not names.
    pub fn parse(s: &str) -> Result<Self, DosError> {
        let (base, ext) = split(s).ok_or(DosError::PathNotFound)?;
        if base.len() > 8 || ext.len() > 3 || !base.iter().chain(&ext).all(|&c| legal(c)) {
            return Err(DosError::PathNotFound);
        }
        let mut fcb = [b' '; 11];
        fcb[..base.len()].copy_from_slice(&base);
        fcb[8..8 + ext.len()].copy_from_slice(&ext);
        Ok(Self(fcb))
    }

    /// The 11-byte FCB form (`"README  TXT"`).
    pub fn fcb(&self) -> [u8; 11] {
        self.0
    }

    pub fn base(&self) -> &str {
        Self::trim(&self.0[..8])
    }

    pub fn ext(&self) -> &str {
        Self::trim(&self.0[8..])
    }

    fn trim(b: &[u8]) -> &str {
        let end = b.iter().rposition(|&c| c != b' ').map_or(0, |i| i + 1);
        // Names are validated ASCII or high bytes; high bytes aren't UTF-8.
        std::str::from_utf8(&b[..end]).unwrap_or("?")
    }
}

impl fmt::Display for DosName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ext().is_empty() {
            write!(f, "{}", self.base())
        } else {
            write!(f, "{}.{}", self.base(), self.ext())
        }
    }
}

impl fmt::Debug for DosName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DosName({self})")
    }
}

/// A FindFirst pattern in FCB form, `?` matching any byte (including the
/// padding). `*` fills the rest of its field with `?`, so `*` alone matches
/// only names without an extension and `*.*` matches everything.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DosPattern([u8; 11]);

impl DosPattern {
    pub fn parse(s: &str) -> Result<Self, DosError> {
        let (base, ext) = split(s).ok_or(DosError::PathNotFound)?;
        let mut fcb = [b' '; 11];
        Self::fill(&mut fcb[..8], &base)?;
        Self::fill(&mut fcb[8..], &ext)?;
        Ok(Self(fcb))
    }

    /// From an FCB name field (already `?`-expanded or not).
    pub fn from_fcb(fcb: [u8; 11]) -> Self {
        let mut p = fcb;
        let (base, ext) = p.split_at_mut(8);
        for field in [base, ext] {
            if let Some(i) = field.iter().position(|&c| c == b'*') {
                field[i..].fill(b'?');
            }
        }
        Self(p)
    }

    fn fill(field: &mut [u8], src: &[u8]) -> Result<(), DosError> {
        for (i, &c) in src.iter().enumerate() {
            if c == b'*' {
                field[i..].fill(b'?');
                return Ok(());
            }
            if i >= field.len() || !(c == b'?' || legal(c)) {
                return Err(DosError::PathNotFound);
            }
            field[i] = c;
        }
        Ok(())
    }

    pub fn matches(&self, name: &DosName) -> bool {
        self.0
            .iter()
            .zip(&name.0)
            .all(|(&p, &n)| p == b'?' || p == n)
    }

    /// True if the pattern contains no wildcards.
    pub fn is_literal(&self) -> bool {
        !self.0.contains(&b'?')
    }
}

impl fmt::Debug for DosPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DosPattern({:?})", String::from_utf8_lossy(&self.0))
    }
}

/// Does `pattern` (e.g. `FOO*.?X`) match `name`? Invalid patterns match nothing.
pub fn matches(pattern: &str, name: &DosName) -> bool {
    DosPattern::parse(pattern).is_ok_and(|p| p.matches(name))
}

/// Drive-relative absolute path: validated components from the root, with
/// `.`/`..` already resolved. The root is the empty path.
#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DosPath(Vec<DosName>);

impl DosPath {
    pub fn root() -> Self {
        Self::default()
    }

    /// Parse `\A\B.TXT` (`/` also accepted; a leading separator is
    /// optional; no drive letter). `..` above the root is an error.
    pub fn parse(s: &str) -> Result<Self, DosError> {
        let mut p = Self::root();
        for comp in s.split(['\\', '/']) {
            match comp {
                "" | "." => {}
                ".." => {
                    p.0.pop().ok_or(DosError::PathNotFound)?;
                }
                _ => p.0.push(DosName::parse(comp)?),
            }
        }
        Ok(p)
    }

    pub fn from_names(names: Vec<DosName>) -> Self {
        Self(names)
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn components(&self) -> &[DosName] {
        &self.0
    }

    pub fn file_name(&self) -> Option<&DosName> {
        self.0.last()
    }

    /// Parent directory; `None` for the root.
    pub fn parent(&self) -> Option<DosPath> {
        let (_, init) = self.0.split_last()?;
        Some(Self(init.to_vec()))
    }

    pub fn join(&self, name: DosName) -> DosPath {
        let mut p = self.clone();
        p.0.push(name);
        p
    }

    /// True if `self` equals `other` or lies below it.
    pub fn starts_with(&self, other: &DosPath) -> bool {
        self.0.starts_with(&other.0)
    }
}

impl fmt::Display for DosPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("\\");
        }
        for n in &self.0 {
            write!(f, "\\{n}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for DosPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DosPath({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> DosName {
        DosName::parse(s).unwrap()
    }

    #[test]
    fn names_are_validated_and_uppercased() {
        assert_eq!(n("readme.txt").to_string(), "README.TXT");
        assert_eq!(&n("a").fcb(), b"A          ");
        assert_eq!(n("FOO.").to_string(), "FOO");
        assert_eq!(n("12345678.ABC").base(), "12345678");
        assert_eq!(n("$x~1_{}.@#!").ext(), "@#!");
        for bad in [
            "",
            ".",
            "..",
            ".TXT",
            "123456789",
            "A.ABCD",
            "A.B.C",
            "A B",
            "A*",
            "A?",
            "A+B",
            "A,B",
            "A;B",
            "A=B",
            "A[B]",
            "A\"B",
            "A/B",
            "A\\B",
            "A:B",
            "A|B",
            "A<B",
        ] {
            assert_eq!(DosName::parse(bad), Err(DosError::PathNotFound), "{bad:?}");
        }
    }

    #[test]
    fn paths_parse_and_resolve_dots() {
        let p = DosPath::parse("\\games\\.\\x\\..\\doom\\doom.exe").unwrap();
        assert_eq!(p.to_string(), "\\GAMES\\DOOM\\DOOM.EXE");
        assert_eq!(p.components().len(), 3);
        assert_eq!(p.file_name(), Some(&n("DOOM.EXE")));
        assert_eq!(p.parent().unwrap().to_string(), "\\GAMES\\DOOM");
        assert_eq!(
            DosPath::parse("a/b").unwrap(),
            DosPath::parse("\\A\\B\\").unwrap()
        );
        assert!(DosPath::parse("").unwrap().is_root());
        assert!(DosPath::parse("\\").unwrap().is_root());
        assert_eq!(DosPath::root().to_string(), "\\");
        assert_eq!(DosPath::root().parent(), None);
        assert_eq!(DosPath::parse("\\.."), Err(DosError::PathNotFound));
        assert_eq!(DosPath::parse("A\\..\\.."), Err(DosError::PathNotFound));
        assert_eq!(DosPath::parse("C:\\X"), Err(DosError::PathNotFound));
        assert!(p.starts_with(&DosPath::parse("GAMES").unwrap()));
        assert!(!p.starts_with(&DosPath::parse("GAME").unwrap()));
    }

    #[test]
    fn wildcard_table() {
        let table: &[(&str, &str, bool)] = &[
            ("*.*", "README.TXT", true),
            ("*.*", "README", true),
            ("*", "README", true),
            ("*", "README.TXT", false),
            ("*.TXT", "README.TXT", true),
            ("*.TXT", "README.DOC", false),
            ("R*.T*", "README.TXT", true),
            ("README.*", "README", true),
            ("?", "A", true),
            ("?", "AB", false),
            ("??????", "AB", true),
            ("A?C", "ABC", true),
            ("A?C", "AC", false),
            ("FOO*.?X", "FOOBAR.AX", true),
            ("FOO*.?X", "FOO.EX", true),
            ("FOO*.?X", "FOO.X", false),
            ("FOO*.?X", "FOO.ABX", false),
            ("FOO*.?X", "FO.AX", false),
            ("*BAR.TXT", "FOO.TXT", true), // chars after * are ignored
            ("readme.txt", "README.TXT", true),
            ("A*B*C", "AXXX", true),
        ];
        for &(pat, name, want) in table {
            assert_eq!(matches(pat, &n(name)), want, "{pat} vs {name}");
        }
        assert!(!matches("A.B.C", &n("A")));
        assert!(DosPattern::parse("A.TXT").unwrap().is_literal());
        assert!(!DosPattern::parse("*.TXT").unwrap().is_literal());
        let fcb = DosPattern::from_fcb(*b"FOO*    T* ");
        assert!(fcb.matches(&n("FOOD.TXT")));
        assert!(!fcb.matches(&n("FOOD.EXE")));
    }
}
