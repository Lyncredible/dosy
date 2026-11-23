use super::*;
use crate::bus::FlatBus;
use crate::machine::{Machine, MachineConfig, RunExit};
use std::sync::Arc;

fn cpu_at(ip: u16) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.far_jump(0x1000, ip);
    cpu
}

#[test]
fn ring_buffer_wraps_oldest_first() {
    let mut bus = FlatBus::new();
    let mut t = Tracer::new(3);
    assert_eq!(t.entries().count(), 0);
    for ip in 0..5 {
        t.record(&cpu_at(ip), &mut bus);
    }
    let ips: Vec<u16> = t.entries().map(|e| e.ip).collect();
    assert_eq!(ips, [2, 3, 4]);
    // Partially filled.
    let mut t = Tracer::new(4);
    t.record(&cpu_at(7), &mut bus);
    t.record(&cpu_at(8), &mut bus);
    assert_eq!(t.entries().map(|e| e.ip).collect::<Vec<_>>(), [7, 8]);
    // Depth 0: no history.
    let mut t = Tracer::new(0);
    t.record(&cpu_at(1), &mut bus);
    assert_eq!(t.entries().count(), 0);
}

struct VecSink(Arc<Mutex<Vec<String>>>);
impl TraceSink for VecSink {
    fn line(&mut self, line: &str) {
        self.0.lock().unwrap().push(line.to_owned());
    }
}

#[test]
fn sink_gets_every_instruction() {
    let mut bus = FlatBus::new();
    bus.load(linear(0x1000, 0), &[0xB8, 0x34, 0x12, 0xCD, 0x21]);
    let lines = Arc::new(Mutex::new(Vec::new()));
    let mut t = Tracer::new(0).with_sink(Box::new(VecSink(lines.clone())));
    let mut cpu = cpu_at(0);
    cpu.regs.set_ax(0x4C00);
    t.record(&cpu, &mut bus);
    cpu.ip = 3;
    t.record(&cpu, &mut bus);
    let lines = lines.lock().unwrap();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("1000:0000  B8 34 12"), "{}", lines[0]);
    assert!(lines[0].contains("mov ax,1234h"));
    assert!(lines[0].contains("AX=4C00"));
    assert!(lines[1].contains("int 21h"));
    let e = TraceEntry::capture(&cpu, &mut bus);
    assert_eq!(
        int_call(&e),
        "INT 21h AX=4C00 BX=0000 CX=0000 DX=0000 SI=0000 DI=0000 DS=0000 ES=0000 from 1000:0003"
    );
}

#[test]
fn warn_once_dedupes_by_category_and_key() {
    let mut w = WarnOnce::default();
    assert!(w.first("port", 0x3F8));
    assert!(!w.first("port", 0x3F8));
    assert!(w.first("port", 0x2F8));
    assert!(w.first("int21", 0x3F8));
    // The global registry reports only the first occurrence.
    let firsts: Vec<bool> = (0..3)
        .map(|_| warn_once("test-dedupe", 42, || String::from("x")))
        .collect();
    assert_eq!(firsts, [true, false, false]);
    assert!(warn_once("test-dedupe", 43, String::new));
}

#[test]
fn disasm_common_opcodes() {
    let d = |b: &[u8]| disasm::disasm(b, 0x100);
    assert_eq!(d(&[0xB8, 0x34, 0x12]), ("mov ax,1234h".into(), 3));
    assert_eq!(d(&[0xCD, 0x21]), ("int 21h".into(), 2));
    assert_eq!(d(&[0xEB, 0x03]), ("jmp short 105h".into(), 2));
    assert_eq!(d(&[0x74, 0xFE]), ("je 100h".into(), 2));
    assert_eq!(d(&[0xE8, 0x00, 0x01]), ("call 203h".into(), 3));
    assert_eq!(
        d(&[0x26, 0xC7, 0x40, 0x10, 0xEF, 0xBE]),
        ("mov word es:[bx+si+10h],BEEFh".into(), 6)
    );
    assert_eq!(d(&[0x8B, 0x46, 0xFE]), ("mov ax,word [bp-2h]".into(), 3));
    assert_eq!(d(&[0x83, 0xC4, 0xFE]), ("add sp,FFFEh".into(), 3));
    assert_eq!(d(&[0xF3, 0xA4]), ("rep movsb".into(), 2));
    assert_eq!(d(&[0xFE, 0x38, 0x21, 0x01]), ("callback 121h".into(), 4));
    assert_eq!(
        d(&[0xFF, 0x1E, 0x00, 0x02]),
        ("call far word [200h]".into(), 4)
    );
    assert_eq!(d(&[0xD1, 0xE0]), ("shl ax,1".into(), 2));
    assert_eq!(d(&[0x0F, 0x00]), ("db 0Fh".into(), 1));
}

#[test]
fn fault_report_format() {
    let mut m = Machine::new(MachineConfig::default());
    m.hw.load_rom(linear(0x1000, 0), &[0x90, 0xB0, 0x07, 0xFF, 0xFF]);
    m.cpu.far_jump(0x1000, 0);
    m.cpu.sregs.set(Seg::Ss, 0x2000);
    m.cpu.regs.set_sp(0x100);
    m.tracer = Some(Box::new(Tracer::new(16)));
    let exit = m.run_for(100);
    assert!(matches!(exit, RunExit::Fault(_)));
    let report = m.fault_report(exit);
    let lines: Vec<&str> = report.lines().collect();
    assert_eq!(lines[0], "invalid opcode FFh at 1000:0003");
    assert!(lines[1].starts_with("AX=0007 BX=0000"), "{}", lines[1]);
    assert!(lines[2].starts_with("CS=1000 DS=") && lines[2].contains("IP=0003"));
    assert!(lines[3].starts_with("1000:0003: FF FF 00"), "{}", lines[3]);
    assert_eq!(lines[4], "  db FFh");
    assert_eq!(lines[5], "last 3 instructions:");
    assert!(lines[6].contains("nop"));
    assert!(lines[7].contains("mov al,7h"));
    assert!(lines[8].starts_with("  1000:0003"));

    // Without a tracer the report says how to get history.
    let mut m = Machine::new(MachineConfig::default());
    m.hw.load_rom(linear(0x1000, 0), &[0xFF, 0xFF]);
    m.cpu.far_jump(0x1000, 0);
    let exit = m.run_for(10);
    assert!(
        m.fault_report(exit)
            .ends_with("(no instruction history; run with --trace)\n")
    );
}
