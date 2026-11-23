//! Disassembler-lite for trace lines and fault reports: Intel syntax for
//! the 8086/186 opcodes programs commonly use; anything else is `db XXh`.

const REG8: [&str; 8] = ["al", "cl", "dl", "bl", "ah", "ch", "dh", "bh"];
const REG16: [&str; 8] = ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di"];
const SREG: [&str; 8] = ["es", "cs", "ss", "ds", "fs", "gs", "?", "?"];
const EA: [&str; 8] = ["bx+si", "bx+di", "bp+si", "bp+di", "si", "di", "bp", "bx"];
const ALU: [&str; 8] = ["add", "or", "adc", "sbb", "and", "sub", "xor", "cmp"];
const SHIFT: [&str; 8] = ["rol", "ror", "rcl", "rcr", "shl", "shr", "sal", "sar"];
const GRP3: [&str; 8] = ["test", "test", "not", "neg", "mul", "imul", "div", "idiv"];
const JCC: [&str; 16] = [
    "jo", "jno", "jb", "jae", "je", "jne", "jbe", "ja", "js", "jns", "jp", "jnp", "jl", "jge",
    "jle", "jg",
];

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> u8 {
        let v = self.b.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        v
    }
    fn u16(&mut self) -> u16 {
        u16::from_le_bytes([self.u8(), self.u8()])
    }
}

fn hex16(v: u16) -> String {
    format!("{v:X}h")
}

/// Decoded ModRM: `reg` field and the r/m operand text.
fn modrm(r: &mut Reader, seg: &str, wide: bool) -> (usize, String) {
    let m = r.u8();
    let (md, reg, rm) = (m >> 6, ((m >> 3) & 7) as usize, (m & 7) as usize);
    if md == 3 {
        let name = if wide { REG16[rm] } else { REG8[rm] };
        return (reg, name.to_string());
    }
    let size = if wide { "word" } else { "byte" };
    let addr = match md {
        0 if rm == 6 => hex16(r.u16()),
        0 => EA[rm].to_string(),
        1 => {
            let d = r.u8() as i8;
            if d < 0 {
                format!("{}-{:X}h", EA[rm], -(d as i16))
            } else {
                format!("{}+{:X}h", EA[rm], d)
            }
        }
        _ => format!("{}+{}", EA[rm], hex16(r.u16())),
    };
    (reg, format!("{size} {seg}[{addr}]"))
}

/// Disassemble the instruction at `bytes` (IP = `ip`, for jump targets).
/// Returns the text and its length in bytes.
pub fn disasm(bytes: &[u8], ip: u16) -> (String, usize) {
    let mut r = Reader { b: bytes, pos: 0 };
    let mut seg = String::new();
    let mut prefix = String::new();
    let op = loop {
        match r.u8() {
            p @ (0x26 | 0x2E | 0x36 | 0x3E) => seg = format!("{}:", SREG[((p >> 3) & 3) as usize]),
            0x64 => seg = "fs:".into(),
            0x65 => seg = "gs:".into(),
            0xF0 => prefix.push_str("lock "),
            0xF2 => prefix.push_str("repne "),
            0xF3 => prefix.push_str("rep "),
            op => break op,
        }
        if r.pos >= bytes.len() {
            return ("(prefixes)".into(), r.pos);
        }
    };
    let rel = |r: &Reader, d: i16| hex16(ip.wrapping_add(r.pos as u16).wrapping_add(d as u16));
    let text = match op {
        0x00..=0x3F if op & 7 < 6 => {
            let name = ALU[(op >> 3) as usize];
            match op & 7 {
                0 | 1 => {
                    let w = op & 1 != 0;
                    let (reg, rm) = modrm(&mut r, &seg, w);
                    format!("{name} {rm},{}", if w { REG16[reg] } else { REG8[reg] })
                }
                2 | 3 => {
                    let w = op & 1 != 0;
                    let (reg, rm) = modrm(&mut r, &seg, w);
                    format!("{name} {},{rm}", if w { REG16[reg] } else { REG8[reg] })
                }
                4 => format!("{name} al,{:X}h", r.u8()),
                _ => format!("{name} ax,{}", hex16(r.u16())),
            }
        }
        0x06 | 0x0E | 0x16 | 0x1E => format!("push {}", SREG[(op >> 3) as usize]),
        0x07 | 0x17 | 0x1F => format!("pop {}", SREG[(op >> 3) as usize]),
        0x27 => "daa".into(),
        0x2F => "das".into(),
        0x37 => "aaa".into(),
        0x3F => "aas".into(),
        0x40..=0x47 => format!("inc {}", REG16[(op & 7) as usize]),
        0x48..=0x4F => format!("dec {}", REG16[(op & 7) as usize]),
        0x50..=0x57 => format!("push {}", REG16[(op & 7) as usize]),
        0x58..=0x5F => format!("pop {}", REG16[(op & 7) as usize]),
        0x60 => "pusha".into(),
        0x61 => "popa".into(),
        0x68 => format!("push {}", hex16(r.u16())),
        0x6A => format!("push {:X}h", r.u8()),
        0x70..=0x7F => {
            let d = r.u8() as i8 as i16;
            format!("{} {}", JCC[(op & 0xF) as usize], rel(&r, d))
        }
        0x80..=0x83 => {
            let w = op & 1 != 0;
            let (reg, rm) = modrm(&mut r, &seg, w);
            let imm = match op {
                0x81 => hex16(r.u16()),
                0x83 => hex16(r.u8() as i8 as u16),
                _ => format!("{:X}h", r.u8()),
            };
            format!("{} {rm},{imm}", ALU[reg])
        }
        0x84..=0x8B => {
            let w = op & 1 != 0;
            let (reg, rm) = modrm(&mut r, &seg, w);
            let reg = if w { REG16[reg] } else { REG8[reg] };
            match op {
                0x84 | 0x85 => format!("test {rm},{reg}"),
                0x86 | 0x87 => format!("xchg {rm},{reg}"),
                0x88 | 0x89 => format!("mov {rm},{reg}"),
                _ => format!("mov {reg},{rm}"),
            }
        }
        0x8C | 0x8E => {
            let (reg, rm) = modrm(&mut r, &seg, true);
            if op == 0x8C {
                format!("mov {rm},{}", SREG[reg])
            } else {
                format!("mov {},{rm}", SREG[reg])
            }
        }
        0x8D | 0xC4 | 0xC5 => {
            let (reg, rm) = modrm(&mut r, &seg, true);
            let name = match op {
                0x8D => "lea",
                0xC4 => "les",
                _ => "lds",
            };
            format!("{name} {},{rm}", REG16[reg])
        }
        0x8F => format!("pop {}", modrm(&mut r, &seg, true).1),
        0x90 => "nop".into(),
        0x91..=0x97 => format!("xchg ax,{}", REG16[(op & 7) as usize]),
        0x98 => "cbw".into(),
        0x99 => "cwd".into(),
        0x9A | 0xEA => {
            let off = r.u16();
            let s = r.u16();
            let name = if op == 0x9A { "call" } else { "jmp" };
            format!("{name} {s:04X}:{off:04X}")
        }
        0x9B => "wait".into(),
        0x9C => "pushf".into(),
        0x9D => "popf".into(),
        0x9E => "sahf".into(),
        0x9F => "lahf".into(),
        0xA0..=0xA3 => {
            let a = format!("{seg}[{}]", hex16(r.u16()));
            match op {
                0xA0 => format!("mov al,{a}"),
                0xA1 => format!("mov ax,{a}"),
                0xA2 => format!("mov {a},al"),
                _ => format!("mov {a},ax"),
            }
        }
        0xA4..=0xA7 | 0xAA..=0xAF => {
            let names = ["movs", "cmps", "", "stos", "lods", "scas"];
            let i = ((op - 0xA4) >> 1) as usize;
            format!(
                "{prefix}{}{}",
                names[i],
                if op & 1 != 0 { 'w' } else { 'b' }
            )
        }
        0xA8 => format!("test al,{:X}h", r.u8()),
        0xA9 => format!("test ax,{}", hex16(r.u16())),
        0xB0..=0xB7 => format!("mov {},{:X}h", REG8[(op & 7) as usize], r.u8()),
        0xB8..=0xBF => format!("mov {},{}", REG16[(op & 7) as usize], hex16(r.u16())),
        0xC2 => format!("ret {}", hex16(r.u16())),
        0xC3 => "ret".into(),
        0xC6 | 0xC7 => {
            let w = op == 0xC7;
            let (_, rm) = modrm(&mut r, &seg, w);
            let imm = if w {
                hex16(r.u16())
            } else {
                format!("{:X}h", r.u8())
            };
            format!("mov {rm},{imm}")
        }
        0xCA => format!("retf {}", hex16(r.u16())),
        0xCB => "retf".into(),
        0xCC => "int3".into(),
        0xCD => format!("int {:02X}h", r.u8()),
        0xCE => "into".into(),
        0xCF => "iret".into(),
        0xD0..=0xD3 => {
            let (reg, rm) = modrm(&mut r, &seg, op & 1 != 0);
            let count = if op < 0xD2 { "1" } else { "cl" };
            format!("{} {rm},{count}", SHIFT[reg])
        }
        0xD4 => format!("aam {:X}h", r.u8()),
        0xD5 => format!("aad {:X}h", r.u8()),
        0xD7 => "xlat".into(),
        0xE0..=0xE3 => {
            let d = r.u8() as i8 as i16;
            let name = ["loopne", "loope", "loop", "jcxz"][(op & 3) as usize];
            format!("{name} {}", rel(&r, d))
        }
        0xE4 => format!("in al,{:X}h", r.u8()),
        0xE5 => format!("in ax,{:X}h", r.u8()),
        0xE6 => format!("out {:X}h,al", r.u8()),
        0xE7 => format!("out {:X}h,ax", r.u8()),
        0xE8 => {
            let d = r.u16() as i16;
            format!("call {}", rel(&r, d))
        }
        0xE9 => {
            let d = r.u16() as i16;
            format!("jmp {}", rel(&r, d))
        }
        0xEB => {
            let d = r.u8() as i8 as i16;
            format!("jmp short {}", rel(&r, d))
        }
        0xEC => "in al,dx".into(),
        0xED => "in ax,dx".into(),
        0xEE => "out dx,al".into(),
        0xEF => "out dx,ax".into(),
        0xF4 => "hlt".into(),
        0xF5 => "cmc".into(),
        0xF6 | 0xF7 => {
            let w = op == 0xF7;
            let (reg, rm) = modrm(&mut r, &seg, w);
            if reg < 2 {
                let imm = if w {
                    hex16(r.u16())
                } else {
                    format!("{:X}h", r.u8())
                };
                format!("test {rm},{imm}")
            } else {
                format!("{} {rm}", GRP3[reg])
            }
        }
        0xF8 => "clc".into(),
        0xF9 => "stc".into(),
        0xFA => "cli".into(),
        0xFB => "sti".into(),
        0xFC => "cld".into(),
        0xFD => "std".into(),
        0xFE if bytes.get(r.pos) == Some(&0x38) => {
            r.u8();
            format!("callback {}", hex16(r.u16()))
        }
        0xFE | 0xFF => {
            let (reg, rm) = modrm(&mut r, &seg, op == 0xFF);
            match reg {
                0 => format!("inc {rm}"),
                1 => format!("dec {rm}"),
                2 => format!("call {rm}"),
                3 => format!("call far {rm}"),
                4 => format!("jmp {rm}"),
                5 => format!("jmp far {rm}"),
                6 => format!("push {rm}"),
                _ => format!("db {op:02X}h"),
            }
        }
        _ => format!("db {op:02X}h"),
    };
    let text = if prefix.is_empty() || text.starts_with(prefix.as_str()) {
        text
    } else {
        format!("{prefix}{text}")
    };
    (text, r.pos)
}
