#![no_std]
#![no_main]

use panic_halt as _;
use esp_hal::{
    Config,
    uart::{Uart, Config as UartConfig},
    time::{Instant, Duration},
    gpio::{Output, Level, OutputConfig}
};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

const BAUD: u32 = 9_600;
const SID:  u8  = 1;

// Timing Modbus (kasar)
const TURNAROUND_SPINS: u32 = 50_000;
const TIMEOUT_SPINS:    u32 = 200_000;

// Servo timing
const SERVO_PERIOD_US: u32 = 20_000;   // 50 Hz = 20 ms
const SERVO_MIN_US:    u32 = 500;      // ~0°
const SERVO_MAX_US:    u32 = 2500;     // ~120° (tuning kalau perlu)

// ===== [TAMBAHAN] Konfigurasi relay kipas =====
const RELAY_ACTIVE_LOW: bool = true; // ubah ke false jika modul relay aktif-HIGH

// Sudut target (0/60/120)
fn deg_to_pulse_us(deg: i32) -> u32 {
    let d = if deg < 0 { 0 } else if deg > 120 { 120 } else { deg } as u32;
    // linear: 0° -> MIN, 120° -> MAX
    SERVO_MIN_US + (d * (SERVO_MAX_US - SERVO_MIN_US) / 120)
}

#[esp_hal::main]
fn main() -> ! {
    let p = esp_hal::init(Config::default());

    // UART1: TX=GPIO17, RX=GPIO18 (ubah sesuai wiring)
    let mut uart = Uart::new(p.UART1, UartConfig::default().with_baudrate(BAUD))
        .expect("UART1 init failed")
        .with_tx(p.GPIO17)
        .with_rx(p.GPIO18);

    // GPIO4 sebagai pin servo (output)
    let mut servo = Output::new(p.GPIO4, Level::Low, OutputConfig::default());

    // ===== [TAMBAHAN] GPIO10 sebagai relay kipas (start OFF) =====
    let mut fan = Output::new(
        p.GPIO10,
        if RELAY_ACTIVE_LOW { Level::High } else { Level::Low }, // OFF awal
        OutputConfig::default()
    );

    println!("\n=== SHT20 (RS485) + SERVO @GPIO4 + RELAY KIPAS @GPIO10 ===");
    println!("Baudrate: {} bps | Slave ID: {}", BAUD, SID);
    println!("-----------------------------------------------------------");

    // state
    let mut target_deg: i32 = 0;
    let mut last_sensor_poll = Instant::now();

    // ===== [TAMBAHAN] state kipas =====
    let mut fan_on: bool = false;
    
    

    loop {
        // --------- 1) Baca sensor tiap ~1 detik ---------
        if last_sensor_poll.elapsed() >= Duration::from_millis(1000) {
            last_sensor_poll = Instant::now();

            println!("\n[Polling Sensor] -----------------------------");

            // ---------- BACA RH @ 0x0002 ----------
            let mut req = [0u8; 8];
            req[0] = SID;
            req[1] = 0x04;
            req[2..4].copy_from_slice(&0x0002u16.to_be_bytes()); // addr RH
            req[4..6].copy_from_slice(&1u16.to_be_bytes());
            let crc = crc16(&req[..6]);
            req[6] = (crc & 0xFF) as u8;
            req[7] = (crc >> 8) as u8;

            let _ = uart.write(&req);
            let _ = uart.flush();
            short_spin(TURNAROUND_SPINS);

            let mut rx = [0u8; 32];
            let mut n = 0usize;
            let mut spins = 0u32;
            while spins < TIMEOUT_SPINS && n < rx.len() {
                let mut b = [0u8; 1];
                match uart.read(&mut b) {
                    Ok(1) => { rx[n] = b[0]; n += 1; if n >= 7 { break; } }
                    _ => { short_spin(1_000); spins += 1; }
                }
            }

            let mut rh_opt: Option<f32> = None;
            if n >= 7 && (rx[1] & 0x80) == 0 && rx[2] == 2 && check_crc(&rx[..n]) {
                let raw_rh = u16::from_be_bytes([rx[3], rx[4]]);
                let rh = raw_rh as f32 / 10.0;
                rh_opt = Some(rh);
                println!("✅ RH = {:.1} %", rh);
            } else {
                println!("⚠️  No/invalid reply for 0x0002 (RH)");
            }

            // ---------- (opsional) BACA T @ 0x0001 ----------
            req[2..4].copy_from_slice(&0x0001u16.to_be_bytes());
            let crc2 = crc16(&req[..6]);
            req[6] = (crc2 & 0xFF) as u8;
            req[7] = (crc2 >> 8) as u8;

            let _ = uart.write(&req);
            let _ = uart.flush();
            short_spin(TURNAROUND_SPINS);

            n = 0; spins = 0;
            while spins < TIMEOUT_SPINS && n < rx.len() {
                let mut b = [0u8; 1];
                match uart.read(&mut b) {
                    Ok(1) => { rx[n] = b[0]; n += 1; if n >= 7 { break; } }
                    _ => { short_spin(1_000); spins += 1; }
                }
            }
            if n >= 7 && (rx[1] & 0x80) == 0 && rx[2] == 2 && check_crc(&rx[..n]) {
                let raw_t = u16::from_be_bytes([rx[3], rx[4]]);
                println!("🌡️  T  = {:.1} °C", raw_t as f32 / 10.0);
            } else {
                println!("⚠️  No/invalid reply for 0x0001 (Temp)");
            }

            // --------- Update target_deg berdasar RH ---------
            if let Some(rh) = rh_opt {
                target_deg = if rh > 70.0 { 120 } else if rh < 60.0 { 0 } else { 60 };

                // ===== [TAMBAHAN] Logika kipas dengan hysteresis =====
                if rh > 80.0 { fan_on = true; }
                if rh < 80.0 { fan_on = false; }

                // ===== [TAMBAHAN] Tulis state ke pin relay =====
                if RELAY_ACTIVE_LOW {
                    if fan_on { fan.set_low(); } else { fan.set_high(); }
                } else {
                    if fan_on { fan.set_high(); } else { fan.set_low(); }
                }

                println!("🔧 Servo target → {}°", target_deg);
                println!("💨 Fan state     → {}", if fan_on { "ON" } else { "OFF" });
                println!("-----------------------------------------------");
            }
        }

        // --------- 2) Satu periode PWM (50 Hz) untuk servo ---------
        let pulse = deg_to_pulse_us(target_deg);
        // HIGH selama pulse µs
        servo.set_high();
        sleep_us(pulse);
        // LOW sisa periode
        servo.set_low();
        sleep_us(SERVO_PERIOD_US - pulse);
        // Ulang loop → hasilnya 50 Hz stabil
    }
}

// ===== Timing helpers =====
fn sleep_us(us: u32) {
    // pakai Instant + busy loop
    let start = Instant::now();
    let target = Duration::from_micros(us as u64);
    while start.elapsed() < target {
        core::hint::spin_loop();
    }
}

fn short_spin(iter: u32) { for _ in 0..iter { core::hint::spin_loop(); } }

// ===== CRC Modbus =====
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
