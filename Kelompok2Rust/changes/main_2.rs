#![no_std]
#![no_main]

use panic_halt as _;
use esp_hal::uart::{Uart, Config as UartConfig};
use esp_hal::Blocking;
use esp_hal::Config;
use esp_println::{println, print};

esp_bootloader_esp_idf::esp_app_desc!();

const BAUD: u32 = 9_600;
const SID:  u8  = 1;
const TURNAROUND_SPINS: u32 = 50_000;   // ~5 ms @9600 (kasar)
const TIMEOUT_SPINS:    u32 = 200_000;  // timeout baca (kasar)

// ==== ENTRY ====
#[esp_hal::main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());

    let mut uart = Uart::new(p.UART1, UartConfig::default().with_baudrate(BAUD))
        .expect("UART1 init failed")
        .with_tx(p.GPIO17)  // TX = GPIO17
        .with_rx(p.GPIO18); // RX = GPIO18

    // println!("\n=== SHT20 (RS485) single-register read: FC=0x04 ===");

    // Baca RH @ 0x0001 (skala 0.1)
    if let Some((raw_rh, _n)) = read_one(&mut uart, SID, 0x04, 0x0002) {
        let rh = raw_rh as f32 / 10.0;
        println!("RH = {:.1} %  (raw=0x{:04X})", rh, raw_rh);
    } else {
        println!("No/invalid reply for 0x0001 (RH)");
    }

    // Baca Temp @ 0x0002 (skala 0.1)
    if let Some((raw_t, _n)) = read_one(&mut uart, SID, 0x04, 0x0001) {
        let t = raw_t as f32 / 10.0;
        println!("T  = {:.1} °C (raw=0x{:04X})", t, raw_t);
    } else {
        println!("No/invalid reply for 0x0002 (Temp)");
    }

    loop {}
}

// ==== Kirim FC=0x04, qty=1, lalu baca minimal 7 byte ====
fn read_one(uart: &mut Uart<'_, Blocking>, sid: u8, fc: u8, reg: u16) -> Option<(u16, usize)> {
    // [ID][FC][ADDR_H][ADDR_L][CNT_H][CNT_L][CRC_L][CRC_H]
    let mut req = [0u8; 8];
    req[0] = sid;
    req[1] = fc;
    req[2..4].copy_from_slice(&reg.to_be_bytes());
    req[4..6].copy_from_slice(&1u16.to_be_bytes());
    let crc = crc16(&req[..6]);
    req[6] = (crc & 0xFF) as u8;      // CRC Lo
    req[7] = (crc >> 8) as u8;        // CRC Hi

    // print_hex("TX", &req);
    let _ = uart.write(&req);
    let _ = uart.flush();

    // Silent interval supaya transceiver auto-direction “lepas” ke RX
    short_spin(TURNAROUND_SPINS);

    // Baca minimal 7 byte (ID+FC+BC+DATA2+CRC2) dengan polling 1-byte
    let mut rx = [0u8; 32];
    let mut n = 0usize;
    let mut spins = 0u32;

    while spins < TIMEOUT_SPINS && n < rx.len() {
        let mut b = [0u8; 1];
        match uart.read(&mut b) {
            Ok(1) => {
                rx[n] = b[0];
                n += 1;
                if n >= 7 { break; } // frame minimal untuk qty=1
            }
            _ => {
                short_spin(1_000); // napas singkat (tanpa Delay)
                spins += 1;
            }
        }
    }

    if n == 0 {
        println!("(timeout/no byte)");
        return None;
    }
    // print_hex("RX", &rx[..n]);


    // Exception?
    if n >= 5 && (rx[1] & 0x80) != 0 {
        println!("Exception: {}", rx[2]);
        return None;
    }

    // Validasi dasar + CRC
    if n < 7 || rx[0] != sid || rx[1] != fc || rx[2] != 2 { return None; }
    if !check_crc(&rx[..n]) { println!("CRC mismatch"); return None; }

    let raw = u16::from_be_bytes([rx[3], rx[4]]);
    Some((raw, n))
}

// ==== Utils ====
#[inline(always)]
fn short_spin(iter: u32) { for _ in 0..iter { core::hint::spin_loop(); } }

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if (crc & 1) != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}
fn check_crc(frame: &[u8]) -> bool {
    if frame.len() < 3 { return false; }
    let calc = crc16(&frame[..frame.len() - 2]);
    frame[frame.len() - 2] == (calc & 0xFF) as u8 && frame[frame.len() - 1] == (calc >> 8) as u8
}
fn print_hex(tag: &str, data: &[u8]) {
    print!("{tag} ({}):", data.len());
    for b in data { print!(" {:02X}", b); }
    println!();
}
