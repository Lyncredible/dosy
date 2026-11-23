use super::*;
use crate::bios;
use crate::hw::Hardware;

fn hw() -> Hardware {
    let mut hw = Hardware::new();
    bios::install(&mut hw);
    hw
}

/// Call INT 10h with the given registers; returns the CPU afterwards.
fn int(hw: &mut Hardware, ax: u16, bx: u16, cx: u16, dx: u16) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.regs.set_ax(ax);
    cpu.regs.set_bx(bx);
    cpu.regs.set_cx(cx);
    cpu.regs.set_dx(dx);
    int10(&mut cpu, hw);
    cpu
}

fn cell(hw: &mut Hardware, col: u32, row: u32) -> (u8, u8) {
    let a = 0xB8000 + (row * 80 + col) * 2;
    (hw.read8(a), hw.read8(a + 1))
}

fn crtc_cursor(hw: &Hardware) -> u16 {
    u16::from_be_bytes([hw.vga.crtc[0x0E], hw.vga.crtc[0x0F]])
}

#[test]
fn mode3_clears_screen_and_programs_registers() {
    let mut hw = hw();
    hw.write16(0xB8000 + 500, 0x1234);
    int(&mut hw, 0x0003, 0, 0, 0);
    for off in (0..0x8000).step_by(2) {
        assert_eq!(hw.read16(0xB8000 + off), 0x0720, "{off:x}");
    }
    assert_eq!(hw.vga.crtc, MODE_80.crtc);
    assert_eq!(hw.vga.seq[1..], MODE_80.seq);
    assert_eq!(hw.vga.gc, MODE_80.gc);
    assert_eq!(hw.vga.ac[..20], MODE_80.ac);
    assert_eq!(hw.vga.misc, 0x67);
    assert_eq!(bda::read8(&mut hw, bda::VIDEO_MODE), 3);
    assert_eq!(bda::read16(&mut hw, bda::VIDEO_COLS), 80);
    assert_eq!(bda::read8(&mut hw, bda::VIDEO_ROWS), 24);
    let a = 2 * 0x10000 + 0x41 * 32;
    assert_eq!(hw.vga.vram[a..a + 16], font::FONT_8X16[0x41 * 16..0x42 * 16]);
    let t = hw.vga.render().text.unwrap();
    assert_eq!((t.cols, t.rows), (80, 25));

    // Bit 7 keeps memory.
    hw.write16(0xB8000, 0x1F41);
    int(&mut hw, 0x0083, 0, 0, 0);
    assert_eq!(hw.read16(0xB8000), 0x1F41);
    assert_eq!(int(&mut hw, 0x0F00, 0, 0, 0).regs.ax(), 0x5083);
}

#[test]
fn other_modes() {
    let mut hw = hw();
    int(&mut hw, 0x0001, 0, 0, 0);
    assert_eq!(bda::read16(&mut hw, bda::VIDEO_COLS), 40);
    assert_eq!(hw.vga.render().text.unwrap().cols, 40);

    int(&mut hw, 0x0007, 0, 0, 0);
    assert_eq!(bda::read16(&mut hw, bda::CRTC_BASE), 0x3B4);
    assert_eq!(bda::read16(&mut hw, bda::EQUIPMENT) & 0x30, 0x30);
    assert_eq!(hw.vga.gc[6], 0x0A);
    assert_eq!(hw.read16(0xB0000), 0x0720);
    assert_eq!(hw.vga.dac[0x18], [0x3F; 3]);

    hw.write8(0xA0000, 0x55);
    int(&mut hw, 0x0013, 0, 0, 0);
    assert_eq!(bda::read8(&mut hw, bda::VIDEO_MODE), 0x13);
    assert_eq!(bda::read16(&mut hw, bda::EQUIPMENT) & 0x30, 0x20);
    assert_eq!((hw.vga.misc, hw.vga.seq[4], hw.vga.gc[5]), (0x63, 0x0E, 0x40));
    assert_eq!((hw.vga.ac[0x10], hw.vga.crtc[0x14]), (0x41, 0x40));
    assert_eq!(hw.read8(0xA0000), 0);
    assert_eq!(hw.vga.dac[1], [0, 0, 0x2A]);
    assert_eq!(hw.vga.dac[31], [0x3F; 3]);
    assert_eq!(hw.vga.dac[32], [0, 0, 0x3F]);
    assert_eq!(hw.vga.dac[40], [0x3F, 0, 0]);
    assert_eq!(hw.vga.dac[248], [0; 3]);

    // Unsupported modes are ignored.
    int(&mut hw, 0x006A, 0, 0, 0);
    assert_eq!(bda::read8(&mut hw, bda::VIDEO_MODE), 0x13);
}

#[test]
fn cursor_in_bda_and_crtc() {
    let mut hw = hw();
    int(&mut hw, 0x0200, 0, 0, 0x050A); // row 5, col 10
    assert_eq!(bda::read16(&mut hw, bda::CURSOR_POS), 0x050A);
    assert_eq!(crtc_cursor(&hw), 5 * 80 + 10);
    let cpu = int(&mut hw, 0x0300, 0, 0, 0);
    assert_eq!((cpu.regs.dx(), cpu.regs.cx()), (0x050A, 0x0607));

    int(&mut hw, 0x0100, 0, 0x0607, 0);
    assert_eq!((hw.vga.crtc[0x0A], hw.vga.crtc[0x0B]), (0x0D, 0x0E));
    int(&mut hw, 0x0100, 0, 0x0007, 0); // block
    assert_eq!((hw.vga.crtc[0x0A], hw.vga.crtc[0x0B]), (0x00, 0x0F));
    int(&mut hw, 0x0100, 0, 0x2000, 0);
    assert_ne!(hw.vga.crtc[0x0A] & 0x20, 0);
    assert_eq!(hw.vga.render().text.unwrap().cursor, None);

    // Page 1: start address and its own cursor.
    int(&mut hw, 0x0200, 0x0100, 0, 0x0102);
    assert_eq!(crtc_cursor(&hw), 5 * 80 + 10, "page 0 still active");
    int(&mut hw, 0x0501, 0, 0, 0);
    assert_eq!(bda::read8(&mut hw, bda::VIDEO_PAGE), 1);
    assert_eq!(u16::from_be_bytes([hw.vga.crtc[0x0C], hw.vga.crtc[0x0D]]), 0x800);
    assert_eq!(crtc_cursor(&hw), 0x800 + 80 + 2);
}

#[test]
fn teletype_wraps_and_scrolls() {
    let mut hw = hw();
    for &c in b"Hi\r\nthere\x08!" {
        int(&mut hw, 0x0E00 | c as u16, 0, 0, 0);
    }
    assert_eq!(cell(&mut hw, 1, 0).0, b'i');
    assert_eq!(cell(&mut hw, 4, 1).0, b'!');
    assert_eq!(cursor(&mut hw, 0), (5, 1));

    // Bottom-right: the char wraps and scrolls the screen up.
    set_cursor(&mut hw, 0, 79, 24);
    hw.write8(0xB8000 + (24 * 80 + 79) * 2 + 1, 0x2F);
    teletype(&mut hw, b'X', 0, 0);
    assert_eq!(cell(&mut hw, 79, 23), (b'X', 0x2F));
    assert_eq!(cell(&mut hw, 79, 24).0, b' ');
    assert_eq!(cell(&mut hw, 4, 0).0, b'!', "row 1 moved to row 0");
    assert_eq!(cursor(&mut hw, 0), (0, 24));
    assert_eq!(crtc_cursor(&hw), 24 * 80);

    // LF on the last row scrolls with the attribute under the cursor.
    hw.write8(0xB8000 + (24 * 80) * 2 + 1, 0x4A);
    teletype(&mut hw, b'\n', 0, 0);
    assert_eq!(cell(&mut hw, 40, 24), (b' ', 0x4A));
    assert_eq!(cursor(&mut hw, 0), (0, 24));
    teletype(&mut hw, 0x07, 0, 0); // BEL: no output
    assert_eq!(cursor(&mut hw, 0), (0, 24));
}

#[test]
fn scroll_window() {
    let mut hw = hw();
    for row in 0..25 {
        for col in 0..80 {
            hw.write8(0xB8000 + (row * 80 + col) * 2, b'A' + row as u8);
        }
    }
    // Up 1 in (10,5)-(20,10) with attr 1Fh.
    int(&mut hw, 0x0601, 0x1F00, 0x050A, 0x0A14);
    assert_eq!(cell(&mut hw, 10, 5).0, b'A' + 6);
    assert_eq!(cell(&mut hw, 20, 9).0, b'A' + 10);
    assert_eq!(cell(&mut hw, 15, 10), (b' ', 0x1F));
    assert_eq!(cell(&mut hw, 9, 5).0, b'A' + 5, "outside unchanged");
    assert_eq!(cell(&mut hw, 21, 10).0, b'A' + 10);
    // Down 2.
    int(&mut hw, 0x0702, 0x2E00, 0x050A, 0x0A14);
    assert_eq!(cell(&mut hw, 10, 5), (b' ', 0x2E));
    assert_eq!(cell(&mut hw, 10, 6), (b' ', 0x2E));
    assert_eq!(cell(&mut hw, 10, 7).0, b'A' + 6);
    // AL=0 clears the window.
    int(&mut hw, 0x0600, 0x0700, 0x0000, 0x184F);
    assert_eq!(cell(&mut hw, 0, 0), (b' ', 0x07));
    assert_eq!(cell(&mut hw, 79, 24), (b' ', 0x07));
}

#[test]
fn write_and_read_chars() {
    let mut hw = hw();
    int(&mut hw, 0x0200, 0, 0, 0x0203);
    int(&mut hw, 0x0951, 0x004E, 3, 0); // 'Q' attr 4Eh x3
    assert_eq!(cell(&mut hw, 5, 2), (b'Q', 0x4E));
    assert_eq!(cell(&mut hw, 6, 2), (b' ', 0x07));
    int(&mut hw, 0x0A52, 0, 1, 0); // 'R', keep attr
    assert_eq!(cell(&mut hw, 3, 2), (b'R', 0x4E));
    assert_eq!(int(&mut hw, 0x0800, 0, 0, 0).regs.ax(), 0x4E52);
    assert_eq!(cursor(&mut hw, 0), (3, 2), "no cursor advance");
}

#[test]
fn palette_and_dac() {
    let mut hw = hw();
    int(&mut hw, 0x1010, 0x0005, 0x0203, 0x0100); // DAC 5 = (1,2,3)
    assert_eq!(hw.vga.dac[5], [1, 2, 3]);
    let cpu = int(&mut hw, 0x1015, 0x0005, 0, 0);
    assert_eq!((cpu.regs.dh(), cpu.regs.ch(), cpu.regs.cl()), (1, 2, 3));
    int(&mut hw, 0x1000, 0x3F01, 0, 0); // palette reg 1 = 3Fh
    assert_eq!(hw.vga.ac[1], 0x3F);
    assert_eq!(int(&mut hw, 0x1007, 0x0001, 0, 0).regs.bh(), 0x3F);
    int(&mut hw, 0x1003, 0x0000, 0, 0); // blink off -> intensity
    assert_eq!(hw.vga.ac[0x10] & 8, 0);

    // Block write/read through ES:DX.
    let mut cpu = Cpu::new();
    cpu.sregs.set(Seg::Es, 0x2000);
    hw.write8(0x20010, 7);
    hw.write8(0x20011, 8);
    hw.write8(0x20012, 9);
    cpu.regs.set_ax(0x1012);
    cpu.regs.set_bx(0x40);
    cpu.regs.set_cx(1);
    cpu.regs.set_dx(0x10);
    int10(&mut cpu, &mut hw);
    assert_eq!(hw.vga.dac[0x40], [7, 8, 9]);
    cpu.regs.set_ax(0x1017);
    cpu.regs.set_dx(0x20);
    int10(&mut cpu, &mut hw);
    assert_eq!(hw.read8(0x20022), 9);
}

#[test]
fn info_functions_and_font_vectors() {
    let mut hw = hw();
    let cpu = int(&mut hw, 0x1A00, 0, 0, 0);
    assert_eq!((cpu.regs.al(), cpu.regs.bx()), (0x1A, 0x0008));
    let cpu = int(&mut hw, 0x1200, 0x0010, 0, 0);
    assert_eq!(cpu.regs.bx(), 0x0003);

    // Vectors point at ROM tables.
    let vec = |hw: &mut Hardware, v: u32| (hw.read16(v * 4 + 2), hw.read16(v * 4));
    assert_eq!(vec(&mut hw, 0x1D), (0xF000, VIDEO_PARAMS));
    assert_eq!(vec(&mut hw, 0x1F), (0xF000, ROM_FONT_8X8_HI));
    assert_eq!(vec(&mut hw, 0x43), (0xF000, ROM_FONT_8X8));
    assert_eq!(hw.read8(linear(0xF000, VIDEO_PARAMS) + 16), 0x71);
    assert_eq!(hw.read8(linear(0xF000, ROM_FONT_8X8_HI)), font::FONT_8X8[128 * 8]);

    // 1130h: ES:BP of the 8x16 font, CX = height, DL = rows - 1.
    let cpu = int(&mut hw, 0x1130, 0x0600, 0, 0);
    assert_eq!((cpu.sregs.get(Seg::Es), cpu.regs.bp()), (0xF000, ROM_FONT_8X16));
    assert_eq!((cpu.regs.cx(), cpu.regs.dl()), (16, 24));
    let a = linear(0xF000, ROM_FONT_8X16) + 0x41 * 16 + 5;
    assert_eq!(hw.read8(a), font::FONT_8X16[0x41 * 16 + 5]);

    // 1Bh state block.
    let mut cpu = Cpu::new();
    cpu.sregs.set(Seg::Es, 0x3000);
    cpu.regs.set_ax(0x1B00);
    int10(&mut cpu, &mut hw);
    assert_eq!(cpu.regs.al(), 0x1B);
    assert_eq!(hw.read8(0x30004), 3);
    assert_eq!(hw.read8(0x30022), 25);
}

#[test]
fn load_8x8_font_gives_50_rows() {
    let mut hw = hw();
    int(&mut hw, 0x1112, 0, 0, 0);
    assert_eq!(bda::read8(&mut hw, bda::VIDEO_ROWS), 49);
    assert_eq!(bda::read8(&mut hw, bda::CHAR_HEIGHT), 8);
    assert_eq!(hw.vga.crtc[9] & 0x1F, 7);
    let a = 2 * 0x10000 + 0x41 * 32;
    assert_eq!(hw.vga.vram[a..a + 8], font::FONT_8X8[0x41 * 8..0x42 * 8]);
    assert_eq!(hw.vga.render().text.unwrap().rows, 50);
    // Teletype now scrolls at row 49.
    set_cursor(&mut hw, 0, 0, 49);
    teletype(&mut hw, b'\n', 0, 0);
    assert_eq!(cursor(&mut hw, 0), (0, 49));
}
