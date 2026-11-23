//! Deterministic host name → 8.3 mapping for one directory (HostDrive,
//! ZipDrive).
//!
//! Like DOSBox: names that are already valid 8.3 (in any case) map to
//! themselves uppercased; the rest become `BASE~N.EXT` (base cut to fit,
//! illegal characters → `_`, spaces and extra dots dropped). Names are
//! processed in sorted host order, so `N` is stable, and a short name is
//! never handed out twice (e.g. `readme.txt` and `README.TXT` on a
//! case-sensitive host: the second becomes `README~1.TXT`).

use std::collections::HashSet;

use crate::DosName;

/// Characters DOS allows besides `A–Z` and `0–9` (high bytes are mapped to
/// `_` since host names are UTF-8).
const SPECIAL: &str = "!#$%&'()-@^_`{}~";

fn clean(s: &str) -> String {
    s.chars()
        .filter(|&c| c != ' ' && c != '.')
        .map(|c| {
            let c = c.to_ascii_uppercase();
            if c.is_ascii_alphanumeric() || SPECIAL.contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Base and extension for the `~N` form.
fn split_long(name: &str) -> (String, String) {
    let trimmed = name.trim_start_matches('.');
    let (base, ext) = match trimmed.rfind('.') {
        Some(i) => (&trimmed[..i], &trimmed[i + 1..]),
        None => (trimmed, ""),
    };
    let mut base = clean(base);
    if base.is_empty() {
        base.push('_');
    }
    let ext: String = clean(ext).chars().take(3).collect();
    (base, ext)
}

/// Map `host_names` (one directory) to unique short names. Returns
/// `(short, host)` pairs in sorted host-name order; names that cannot be
/// mapped (more than 9 999 999 collisions) are skipped.
pub fn map_dir<S: AsRef<str>>(host_names: &[S]) -> Vec<(DosName, String)> {
    let mut sorted: Vec<&str> = host_names.iter().map(|s| s.as_ref()).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let mut taken = HashSet::new();
    let mut short: Vec<Option<DosName>> = vec![None; sorted.len()];

    // Valid 8.3 names keep their name (first one wins on case clashes).
    for (i, name) in sorted.iter().enumerate() {
        if !name.is_ascii() || name.starts_with('.') || name.contains(' ') {
            continue;
        }
        if let Ok(n) = DosName::parse(name) {
            if taken.insert(n) {
                short[i] = Some(n);
            }
        }
    }
    for (i, name) in sorted.iter().enumerate() {
        if short[i].is_some() {
            continue;
        }
        let (base, ext) = split_long(name);
        short[i] = (1..10_000_000u32).find_map(|n| {
            let tail = format!("~{n}");
            let keep = 8usize.saturating_sub(tail.len());
            let b: String = base.chars().take(keep).collect();
            let cand = if ext.is_empty() {
                format!("{b}{tail}")
            } else {
                format!("{b}{tail}.{ext}")
            };
            DosName::parse(&cand).ok().filter(|n| taken.insert(*n))
        });
    }
    sorted
        .into_iter()
        .zip(short)
        .filter_map(|(h, s)| Some((s?, h.to_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(names: &[&str]) -> Vec<(String, String)> {
        map_dir(names)
            .into_iter()
            .map(|(s, h)| (h, s.to_string()))
            .collect()
    }

    #[test]
    fn short_and_mixed_case_names_keep_their_name() {
        assert_eq!(
            map(&["readme.txt", "Tool.Exe", "A"]),
            [
                ("A", "A"),
                ("Tool.Exe", "TOOL.EXE"),
                ("readme.txt", "README.TXT")
            ]
            .map(|(a, b)| (a.to_string(), b.to_string()))
        );
    }

    #[test]
    fn long_names_get_numbered_aliases() {
        let m = map(&[
            "Long File Name.text",
            "longfilename.txt",
            "LongFileNameB.txt",
            "my.archive.tar.gz",
            ".hidden",
            "a+b.c",
            "café.txt",
        ]);
        let get = |h: &str| m.iter().find(|(x, _)| x == h).unwrap().1.clone();
        // Sorted host order: "Long File..." < "LongFileNameB" < "longfilename".
        assert_eq!(get("Long File Name.text"), "LONGFI~1.TEX");
        assert_eq!(get("LongFileNameB.txt"), "LONGFI~1.TXT");
        assert_eq!(get("longfilename.txt"), "LONGFI~2.TXT");
        assert_eq!(get("my.archive.tar.gz"), "MYARCH~1.GZ");
        assert_eq!(get(".hidden"), "HIDDEN~1");
        assert_eq!(get("a+b.c"), "A_B~1.C");
        assert_eq!(get("café.txt"), "CAF_~1.TXT");
    }

    #[test]
    fn case_clashes_and_alias_clashes_stay_unique() {
        let m = map(&["README.TXT", "readme.txt", "README~1.TXT"]);
        let shorts: HashSet<_> = m.iter().map(|(_, s)| s.clone()).collect();
        assert_eq!(shorts.len(), 3);
        assert!(m.contains(&("README.TXT".into(), "README.TXT".into())));
        assert!(m.contains(&("README~1.TXT".into(), "README~1.TXT".into())));
        assert!(m.contains(&("readme.txt".into(), "README~2.TXT".into())));
    }

    #[test]
    fn many_collisions_shrink_the_base() {
        let names: Vec<String> = (0..12).map(|i| format!("longname{i:02}.dat")).collect();
        let m = map_dir(&names);
        assert_eq!(m[8].0.to_string(), "LONGNA~9.DAT");
        assert_eq!(m[9].0.to_string(), "LONGN~10.DAT");
        // Deterministic regardless of input order.
        let mut rev = names.clone();
        rev.reverse();
        assert_eq!(map_dir(&rev), m);
    }
}
