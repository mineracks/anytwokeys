//! The SeedHammer v1 engraver (a MarkingWay marking machine behind an FTDI USB-serial chip),
//! driven the way its own Pi firmware drives it. Desktop only: a phone has no serial port.
//!
//! The strokes come from the composer WASM in the window (`composerProgram`): 10-byte commands
//! in PLATE steps, 126 per millimetre, origin at the plate's top-left. This module adds the jig
//! offset for the plate type and speaks the wire protocol. Everything about that protocol was
//! read from upstream's `driver/mjolnir` and then confirmed byte for byte against the machine on
//! 2026-10-07 (mineracks/seedhammer-v1-companion, docs/architecture/v1-engrave-spec.md):
//!
//!   cancel 0xAF; init 0x00 -> status 0x00; set speeds 0x30 + print,move,xxx (LE16 each, lower
//!   is faster, 1000..30) -> echo; set delays 0x31 pen-down pen-up -> echo; origin 0x21 0x50 ->
//!   0x21 0x00 (homes); query 0x16 -> 0x16 + x,y,z LE24; program 0x60 nbatches(LE16), then the
//!   engraver asks for each 800-byte batch of 80 commands with status 0x60, reports a step with
//!   0x6F, completion with 0x6A, cancel with 0x62/0x65. Command: 0x80 move / 0x00 line + x,y,z
//!   LE24. It asks for one batch more than declared, so the last one is padding (0xFF).
//!
//! Dry run means every command is sent as a move: the head traces the design with the pen up.
//! That is how a layout is checked against a mounted plate before anything is hammered.
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const STEPS_PER_MM: i32 = 126;
const BATCH: usize = 80;
const CMD: usize = 10;

/// Where each plate's top-left corner sits on the bed, in millimetres from home. Upstream keeps
/// this in its Pi platform code (`cmd/controller/platform_rpi.go`), not in the driver: x = 97
/// for every plate, y = 49 for a Square plate (SH-02) so it sits centred in the Large slot, 0
/// for a Large one (SH-03). Verified 2026-10-07: pen-up outlines framed both plate edges exactly.
pub fn jig(plate: u8) -> Result<(i32, i32), String> {
    match plate {
        1 => Ok((97, 49)),
        2 => Ok((97, 0)),
        0 => Err("the small (85x55) plate has no known position on the bed".into()),
        p => Err(format!("unknown plate type {p}")),
    }
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct Port { pub path: String, pub serial: String, pub product: String }

/// The engravers on this machine: FTDI FT232R serial adapters (what the MarkingWay uses).
pub fn find() -> Vec<Port> {
    let mut out = Vec::new();
    let Ok(ports) = serialport::available_ports() else { return out };
    for p in ports {
        if let serialport::SerialPortType::UsbPort(u) = &p.port_type {
            if u.vid == 0x0403 {
                // macOS lists both /dev/tty.* and /dev/cu.*; cu is the one to open
                if p.port_name.starts_with("/dev/tty.") { continue; }
                out.push(Port { path: p.port_name.clone(), serial: u.serial_number.clone().unwrap_or_default(), product: u.product.clone().unwrap_or_else(|| "USB serial".into()) });
            }
        }
    }
    out
}

/// One job: a plate type, its commands (plate steps), and whether the pen stays up.
pub struct Job { pub plate: u8, pub commands: Vec<u8>, pub dry: bool }

pub struct Progress { pub done: usize, pub total: usize }

struct Wire { port: Box<dyn serialport::SerialPort>, cancel: Arc<AtomicBool> }

impl Wire {
    fn write(&mut self, b: &[u8]) -> Result<(), String> { self.port.write_all(b).map_err(|e| format!("engraver write: {e}")) }
    /// Read exactly n bytes, giving the machine up to `wait` to produce them.
    fn read_n(&mut self, n: usize, wait: Duration) -> Result<Vec<u8>, String> {
        let mut got = Vec::with_capacity(n);
        let end = Instant::now() + wait;
        let mut buf = [0u8; 64];
        while got.len() < n {
            if Instant::now() > end { return Err(format!("engraver did not answer ({} of {} bytes)", got.len(), n)); }
            if self.cancel.load(Ordering::Relaxed) { let _ = self.port.write_all(&[0xaf]); return Err("cancelled".into()); }
            match self.port.read(&mut buf[..(n - got.len()).min(64)]) {
                Ok(0) => {}
                Ok(k) => got.extend_from_slice(&buf[..k]),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => return Err(format!("engraver read: {e}")),
            }
        }
        Ok(got)
    }
    fn expect(&mut self, want: &[u8], what: &str) -> Result<(), String> {
        let got = self.read_n(want.len(), Duration::from_secs(20))?;
        if got != want { return Err(format!("{what}: engraver answered {} instead of {}", hex(&got), hex(want))); }
        Ok(())
    }
    fn init(&mut self) -> Result<(), String> {
        self.write(&[0xaf])?;
        std::thread::sleep(Duration::from_millis(300));
        let _ = self.port.clear(serialport::ClearBuffer::Input);
        self.write(&[0x00])?;
        for _ in 0..20 {
            let st = self.read_n(1, Duration::from_secs(10))?[0];
            match st { 0x00 => return Ok(()), 0x65 => self.write(&[0x00])?, _ => {} }
        }
        Err("engraver did not initialise".into())
    }
    fn speeds(&mut self, print: u16, mv: u16) -> Result<(), String> {
        let xxx: u16 = 0xe6;
        self.write(&[0x30, print as u8, (print >> 8) as u8, mv as u8, (mv >> 8) as u8, xxx as u8, (xxx >> 8) as u8])?;
        self.expect(&[0x30], "set speeds")
    }
    fn delays(&mut self) -> Result<(), String> { self.write(&[0x31, 0x14, 0x14])?; self.expect(&[0x31], "set delays") }
    fn origin(&mut self) -> Result<(), String> { self.write(&[0x21, 0x50])?; self.expect(&[0x21, 0x00], "home") }
    /// Stream one program; `on_step` is called per executed command.
    fn program(&mut self, cmds: &[u8], mut on_step: impl FnMut(usize)) -> Result<(), String> {
        let count = cmds.len() / CMD;
        let nbatches = (count + BATCH) / BATCH; // one extra, padded: the engraver asks for it
        if nbatches > 0xffff { return Err("program too large".into()); }
        self.write(&[0x60, nbatches as u8, (nbatches >> 8) as u8])?;
        let mut sent = 0usize;
        let mut steps = 0usize;
        loop {
            let st = self.read_n(1, Duration::from_secs(120))?[0];
            match st {
                0x60 => {
                    if sent >= nbatches * BATCH { continue; }
                    let n = (count - sent.min(count)).min(BATCH);
                    let mut batch = Vec::with_capacity(BATCH * CMD);
                    batch.extend_from_slice(&cmds[sent.min(count) * CMD..(sent.min(count) + n) * CMD]);
                    batch.resize(BATCH * CMD, 0xff);
                    self.write(&batch)?;
                    sent += BATCH;
                }
                0x6f => { steps += 1; on_step(steps); }
                0x6a => return Ok(()),
                0x62 => {}
                0x65 => return Err("cancelled".into()),
                other => return Err(format!("engraver sent status {other:#04x} mid-program")),
            }
        }
    }
    fn move_to(&mut self, x: i32, y: i32) -> Result<(), String> {
        let c = [0x80, x as u8, (x >> 8) as u8, (x >> 16) as u8, y as u8, (y >> 8) as u8, (y >> 16) as u8, 0, 0, 0];
        self.program(&c, |_| {})
    }
}

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ") }

/// Run a job, exactly as the Pi firmware runs one: initialise, warm-up speeds, home, step out
/// and re-home (a false home is possible if the head starts on the switch), park at the safe
/// point beside the plate, set the working speeds, stream the program, slow down, park, home.
pub fn run(port_path: &str, job: &Job, cancel: Arc<AtomicBool>, mut progress: impl FnMut(Progress)) -> Result<(), String> {
    if job.commands.len() % CMD != 0 { return Err("commands are not 10-byte records".into()); }
    let (jx, jy) = jig(job.plate)?;
    // plate steps -> bed steps; a dry run turns every line into a move
    let mut cmds = job.commands.clone();
    for c in cmds.chunks_mut(CMD) {
        if job.dry { c[0] = 0x80; }
        let x = i32::from_le_bytes([c[1], c[2], c[3], 0]) + jx * STEPS_PER_MM;
        let y = i32::from_le_bytes([c[4], c[5], c[6], 0]) + jy * STEPS_PER_MM;
        if !(0..=0xff_ffff).contains(&x) || !(0..=0xff_ffff).contains(&y) { return Err("a stroke lands off the bed".into()); }
        c[1..4].copy_from_slice(&x.to_le_bytes()[..3]);
        c[4..7].copy_from_slice(&y.to_le_bytes()[..3]);
    }
    let port = serialport::new(port_path, 115_200)
        .data_bits(serialport::DataBits::Eight).parity(serialport::Parity::None).stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None).timeout(Duration::from_millis(300))
        .open().map_err(|e| format!("could not open {port_path}: {e}"))?;
    let mut w = Wire { port, cancel: cancel.clone() };
    let total = cmds.len() / CMD;
    progress(Progress { done: 0, total });
    w.init()?;
    w.speeds(300, 300)?;
    w.delays()?;
    w.origin()?;
    w.move_to(5 * STEPS_PER_MM, 5 * STEPS_PER_MM)?;
    w.origin()?;
    let (sx, sy) = (119 * STEPS_PER_MM, 43 * STEPS_PER_MM); // upstream's safe point, clear of the nuts
    w.move_to(sx, sy)?;
    // upstream defaults: MoveSpeed .5, PrintSpeed .1 on a 1000 (slow) .. 30 (fast) scale
    let print_speed = (0.1f32 * 30.0 + 0.9 * 1000.0) as u16;
    let move_speed = (0.5f32 * 30.0 + 0.5 * 1000.0) as u16;
    w.speeds(print_speed, move_speed)?;
    let result = w.program(&cmds, |done| progress(Progress { done: done.min(total), total }));
    // whatever happened, leave the machine parked and homed if it still talks to us
    let _ = w.speeds(300, 300);
    let _ = w.move_to(sx, sy);
    let _ = w.origin();
    result
}

/// A quick jog to prove the machine is alive: out 5 mm and back, pen up. Returns the position
/// the controller reports after the move and after the return.
pub fn jog(port_path: &str) -> Result<String, String> {
    let port = serialport::new(port_path, 115_200).timeout(Duration::from_millis(300)).open().map_err(|e| format!("could not open {port_path}: {e}"))?;
    let mut w = Wire { port, cancel: Arc::new(AtomicBool::new(false)) };
    w.init()?; w.speeds(300, 300)?; w.delays()?; w.origin()?;
    w.move_to(5 * STEPS_PER_MM, 5 * STEPS_PER_MM)?;
    w.write(&[0x16])?;
    let a = w.read_n(10, Duration::from_secs(5))?;
    w.move_to(0, 0)?;
    w.write(&[0x16])?;
    let b = w.read_n(10, Duration::from_secs(5))?;
    let pos = |v: &[u8]| (i32::from_le_bytes([v[1], v[2], v[3], 0]) as f32 / STEPS_PER_MM as f32, i32::from_le_bytes([v[4], v[5], v[6], 0]) as f32 / STEPS_PER_MM as f32);
    let (ax, ay) = pos(&a); let (bx, by) = pos(&b);
    Ok(format!("moved to ({ax:.1}, {ay:.1}) mm and back to ({bx:.1}, {by:.1}) mm"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jig_table_matches_upstream() {
        assert_eq!(jig(1).unwrap(), (97, 49));
        assert_eq!(jig(2).unwrap(), (97, 0));
        assert!(jig(0).is_err());
    }

    #[test]
    fn a_dry_run_sends_only_moves_and_the_offset_is_applied() {
        // one line to (10mm, 5mm) on a Square plate -> after the jig offset it is a move to (107mm, 54mm)
        let mut cmds = vec![0x00u8]; cmds.extend_from_slice(&(10 * STEPS_PER_MM).to_le_bytes()[..3]); cmds.extend_from_slice(&(5 * STEPS_PER_MM).to_le_bytes()[..3]); cmds.extend_from_slice(&[0, 0, 0]);
        let job = Job { plate: 1, commands: cmds, dry: true };
        let (jx, jy) = jig(job.plate).unwrap();
        let mut c = job.commands.clone();
        for ch in c.chunks_mut(CMD) {
            if job.dry { ch[0] = 0x80; }
            let x = i32::from_le_bytes([ch[1], ch[2], ch[3], 0]) + jx * STEPS_PER_MM;
            let y = i32::from_le_bytes([ch[4], ch[5], ch[6], 0]) + jy * STEPS_PER_MM;
            ch[1..4].copy_from_slice(&x.to_le_bytes()[..3]); ch[4..7].copy_from_slice(&y.to_le_bytes()[..3]);
        }
        assert_eq!(c[0], 0x80);
        assert_eq!(i32::from_le_bytes([c[1], c[2], c[3], 0]), 107 * STEPS_PER_MM);
        assert_eq!(i32::from_le_bytes([c[4], c[5], c[6], 0]), 54 * STEPS_PER_MM);
    }
}
