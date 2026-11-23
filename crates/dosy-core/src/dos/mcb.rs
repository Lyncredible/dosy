//! Memory control block (MCB) chain in guest memory.
//!
//! Each block is preceded by a one-paragraph MCB: `+0` type (`'M'`, or `'Z'`
//! for the last), `+1` owner PSP (0 = free), `+3` size in paragraphs (MCB
//! excluded), `+8` 8-byte owner name. Free neighbours are merged during scans.

use super::{ERR_BAD_BLOCK, ERR_MCB_DESTROYED, ERR_NO_MEMORY};
use crate::bus::{Bus, linear};

pub const MCB_MID: u8 = b'M';
pub const MCB_LAST: u8 = b'Z';
/// Owner value of a free block.
pub const FREE: u16 = 0;
/// Owner value of blocks belonging to DOS itself.
pub const OWNER_DOS: u16 = 8;

/// Allocation strategy (INT 21h/58h), low two bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Strategy {
    #[default]
    FirstFit = 0,
    BestFit = 1,
    LastFit = 2,
}

impl Strategy {
    /// From an INT 21h/58h value: bits 0–5 pick the fit (above 2 acts as
    /// last fit, as in DOS); the UMB bits 6–7 don't affect conventional
    /// memory.
    pub fn from_dos(v: u16) -> Self {
        match v & 0x3F {
            0 => Self::FirstFit,
            1 => Self::BestFit,
            _ => Self::LastFit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mcb {
    pub seg: u16,
    pub kind: u8,
    pub owner: u16,
    pub size: u16,
}

impl Mcb {
    /// First paragraph of the block itself.
    pub fn data(&self) -> u16 {
        self.seg + 1
    }
    /// Segment of the next MCB.
    pub fn next(&self) -> u16 {
        self.seg + 1 + self.size
    }
    pub fn is_last(&self) -> bool {
        self.kind == MCB_LAST
    }
}

pub fn read<B: Bus>(bus: &mut B, seg: u16) -> Mcb {
    let a = linear(seg, 0);
    Mcb {
        seg,
        kind: bus.read8(a),
        owner: bus.read16(a + 1),
        size: bus.read16(a + 3),
    }
}

pub fn write<B: Bus>(bus: &mut B, m: &Mcb) {
    let a = linear(m.seg, 0);
    bus.write8(a, m.kind);
    bus.write16(a + 1, m.owner);
    bus.write16(a + 3, m.size);
}

/// Set the 8-byte owner name (NUL-padded, truncated).
pub fn set_name<B: Bus>(bus: &mut B, seg: u16, name: &[u8]) {
    let a = linear(seg, 8);
    for i in 0..8 {
        bus.write8(a + i, name.get(i as usize).copied().unwrap_or(0));
    }
}

/// Create a single free block spanning `first..end` (end exclusive).
pub fn init<B: Bus>(bus: &mut B, first: u16, end: u16) {
    write(
        bus,
        &Mcb {
            seg: first,
            kind: MCB_LAST,
            owner: FREE,
            size: end - first - 1,
        },
    );
    set_name(bus, first, b"");
}

/// Walk the chain from `first`, merging adjacent free blocks. Returns all
/// blocks, or `ERR_MCB_DESTROYED` on a bad type byte or a runaway chain.
pub fn walk<B: Bus>(bus: &mut B, first: u16) -> Result<Vec<Mcb>, u16> {
    let mut out: Vec<Mcb> = Vec::new();
    let mut seg = first;
    loop {
        let m = read(bus, seg);
        // The block (and for 'M' the next MCB) must fit below 1 MiB, so
        // `data()`/`next()` can't overflow on a corrupt size.
        let end = m.seg as u32 + 1 + m.size as u32;
        let bad_end = end > 0xFFFF || (m.kind == MCB_MID && end >= 0xFFFF);
        if (m.kind != MCB_MID && m.kind != MCB_LAST) || m.seg == 0xFFFF || bad_end {
            return Err(ERR_MCB_DESTROYED);
        }
        match out.last_mut() {
            Some(prev) if prev.owner == FREE && m.owner == FREE => {
                prev.size += m.size + 1;
                prev.kind = m.kind;
                write(bus, prev);
            }
            _ => out.push(m),
        }
        if m.is_last() {
            return Ok(out);
        }
        // `next` must move forward inside the 1 MiB space.
        match (m.seg as u32 + 1 + m.size as u32).try_into() {
            Ok(next) if next > seg => seg = next,
            _ => return Err(ERR_MCB_DESTROYED),
        }
    }
}

/// Split `m` so it keeps `size` paragraphs; the rest becomes a free block.
fn split<B: Bus>(bus: &mut B, m: &mut Mcb, size: u16) {
    if m.size > size {
        let rest = Mcb {
            seg: m.seg + 1 + size,
            kind: m.kind,
            owner: FREE,
            size: m.size - size - 1,
        };
        write(bus, &rest);
        set_name(bus, rest.seg, b"");
        m.kind = MCB_MID;
        m.size = size;
    }
}

/// Largest free block in paragraphs.
pub fn largest_free(blocks: &[Mcb]) -> u16 {
    blocks
        .iter()
        .filter(|m| m.owner == FREE)
        .map(|m| m.size)
        .max()
        .unwrap_or(0)
}

/// Allocate `paras` paragraphs for `owner`; returns the data segment.
/// On `ERR_NO_MEMORY` the error carries the largest free block size.
pub fn alloc<B: Bus>(
    bus: &mut B,
    first: u16,
    paras: u16,
    owner: u16,
    strategy: Strategy,
) -> Result<u16, (u16, u16)> {
    let blocks = walk(bus, first).map_err(|e| (e, 0))?;
    let mut fits = blocks.iter().filter(|m| m.owner == FREE && m.size >= paras);
    let pick = match strategy {
        Strategy::FirstFit => fits.next(),
        Strategy::BestFit => fits.min_by_key(|m| m.size),
        Strategy::LastFit => fits.next_back(),
    };
    let Some(&m) = pick else {
        return Err((ERR_NO_MEMORY, largest_free(&blocks)));
    };
    let mut m = m;
    if strategy == Strategy::LastFit && m.size > paras {
        // Take the top of the block; the bottom stays free.
        let top = Mcb {
            seg: m.next() - paras - 1,
            kind: m.kind,
            owner,
            size: paras,
        };
        m.kind = MCB_MID;
        m.size -= paras + 1;
        write(bus, &m);
        write(bus, &top);
        set_name(bus, top.seg, b"");
        return Ok(top.data());
    }
    split(bus, &mut m, paras);
    m.owner = owner;
    write(bus, &m);
    set_name(bus, m.seg, b"");
    Ok(m.data())
}

/// Find the in-chain MCB whose block starts at `data_seg`.
fn find<B: Bus>(bus: &mut B, first: u16, data_seg: u16) -> Result<Mcb, u16> {
    let blocks = walk(bus, first)?;
    blocks
        .into_iter()
        .find(|m| m.data() == data_seg)
        .ok_or(ERR_BAD_BLOCK)
}

/// Free the block at `data_seg`.
pub fn free<B: Bus>(bus: &mut B, first: u16, data_seg: u16) -> Result<(), u16> {
    let mut m = find(bus, first, data_seg)?;
    m.owner = FREE;
    write(bus, &m);
    Ok(())
}

/// Resize the block at `data_seg` to `paras`. If it can't grow that far it
/// grows to the maximum (as DOS 2.1–6.0 do) and returns `ERR_NO_MEMORY`
/// with that size.
pub fn resize<B: Bus>(
    bus: &mut B,
    first: u16,
    data_seg: u16,
    paras: u16,
) -> Result<(), (u16, u16)> {
    let mut m = find(bus, first, data_seg).map_err(|e| (e, 0))?;
    if m.owner == FREE {
        return Err((ERR_BAD_BLOCK, 0));
    }
    // `walk` merged free runs, so at most one free block follows.
    let next = (!m.is_last()).then(|| read(bus, m.next()));
    let avail = match next {
        Some(n) if n.owner == FREE => m.size + 1 + n.size,
        _ => m.size,
    };
    let target = paras.min(avail);
    if target > m.size {
        let n = next.expect("growth needs a free successor");
        m.size = avail;
        m.kind = n.kind;
    }
    split(bus, &mut m, target);
    write(bus, &m);
    if paras > avail {
        return Err((ERR_NO_MEMORY, avail));
    }
    Ok(())
}

/// Free every block owned by `owner` (process termination).
pub fn free_owned<B: Bus>(bus: &mut B, first: u16, owner: u16) -> Result<(), u16> {
    for mut m in walk(bus, first)? {
        if m.owner == owner {
            m.owner = FREE;
            write(bus, &m);
        }
    }
    walk(bus, first).map(|_| ())
}
