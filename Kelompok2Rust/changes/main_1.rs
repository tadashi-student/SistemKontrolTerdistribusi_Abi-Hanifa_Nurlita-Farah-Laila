#![no_std]
#![no_main]

//! ESP32-S3 + RS485 (TTL transceiver) + SHT20 Modbus (RAW RTU)
//! UART1 pins: TX=GPIO17, RX=GPIO18
//!
//! - Tanpa library Modbus; frame dibangun manual (CRC16 Modbus)
//! - Function Code default: FC03 (Holding Registers)
//! - Register default: RH=0x0001, T=0x0002 (skala x10)
//! - Jika transceiver RS485 Anda non-auto-direction (MAX485 manual),
//!   aktifkan kontrol DE/RE: lihat bagian `RS485_DE_RE_PIN` di bawah.
//!
//! NOTE kompatibilitas:
//! - Ditulis mengacu API `esp-hal` 1.0.x (rc/beta) untuk ESP32-S3.
//! - Pastikan `Cargo.toml` mengaktifkan fitur chip & backend print:
//!   [dependencies]
//!   esp-hal     = { version = "1.0.0-rc.0", features = ["esp32s3"] }
//!   esp-println = { version = "0.11", features = ["esp32s3", "uart"] }
//!   # jika perlu app desc untuk bootloader IDF
//!   esp-bootloader-esp-idf = "0.3"
//!
//! - Jika API minor berbeda pada versi Anda, sesuaikan builder `Uart` dan GPIO.

use esp_hal::clock::CpuClock;
use esp_hal::main;
use esp_hal::time::{Duration, Instant};
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_println::println;
use embedded_hal::digital::OutputPin as HalOutputPin;
use esp_hal::gpio::{Level, Output, OutputConfig};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("Panic: {}", info);
    loop {}
}

// Opsional: metadata untuk bootloader ESP-IDF
esp_bootloader_esp_idf::esp_app_desc!();

// ======================= Konstanta & Konfigurasi =======================
const MODBUS_SLAVE_ID: u8 = 1;
const MODBUS_BAUD: u32 = 9_600;

// Function Codes (benar): 0x03=Holding, 0x04=Input.
const FC_READ_HOLDING: u8 = 0x3;
const FC_READ_INPUT: u8 = 0x04;
const FUNCTION_CODE: u8 = FC_READ_INPUT; // ubah ke FC_READ_INPUT jika perlu

// Register default (ubah sesuai datasheet modul Anda)
const REG_HUM: u16 = 0x0000; // %RH * 10
const REG_TMP: u16 = 0x0001; // degC * 10

// Skala pembacaan
const SCALE: f32 = 10.0;

// Timeout & jeda (ms)
const MODBUS_TIMEOUT_MS: u32 = 300; // batas waktu respons
const INTERBYTE_GAP_MS: u32 = 20;   // jeda antar byte -> end-of-frame
const SILENT_INTERVAL_TX_MS: u32 = 4; // ~3.5 char @9600

// RS485 manual direction: set ke Some(pin) jika pakai transceiver manual (DE=/RE)
// Default None untuk modul auto-direction.
// Ganti ke Some(GPIOx) dan atur sebagai output jika perlu.
// Pada esp-hal 1.0, akses pin langsung via `peripherals.GPIOxx`.
// Di bawah kita buat Option yang akan diisi di runtime kalau mau dipakai.

// ======================= CRC16 Modbus =======================
fn modbus_crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            if (crc & 0x0001) != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

// ======================= Util waktu =======================
fn sleep_ms(ms: u32) {
    let start = Instant::now();
    let dur = Duration::from_millis(ms as u64);
    while start.elapsed() < dur {}
}

// ======================= RS485 TX enable =======================
// Jika Anda ingin kontrol DE/RE manual, aktifkan block ini dan isi pin-nya.
// Contoh (pseudo):
// let mut de = Some(peripherals.GPIOX.into_push_pull_output());
// lalu panggil rs485_tx_enable(de.as_mut(), true/false)

fn rs485_tx_enable<P>(pin: Option<&mut P>, en: bool)
where
    P: HalOutputPin,
{
    if let Some(p) = pin {
        // abaikan Result dengan `let _ = ...`
        let _ = if en { HalOutputPin::set_high(p) } else { HalOutputPin::set_low(p) };
    }
}

// ======================= UART helpers =======================
fn uart_write_all(uart: &mut Uart<'_, esp_hal::Blocking>, data: &[u8]) -> Result<(), esp_hal::uart::TxError> {
    uart.write(data)?;
    uart.flush()?;
    Ok(())
}

fn clear_uart_rx(uart: &mut Uart<'_, esp_hal::Blocking>) {
    let mut tmp = [0u8; 64];
    loop {
        match uart.read(&mut tmp) {
            Ok(n) if n > 0 => {
                // buang
            }
            _ => break,
        }
    }
}

// Membaca respons hingga expected_len (jika >0) atau sampai timeout + gap.
fn read_response(
    uart: &mut Uart<'_, esp_hal::Blocking>,
    buf: &mut [u8],
    expected_len: usize,
) -> usize {
    let t0 = Instant::now();
    let mut last_byte = Instant::now();
    let mut idx = 0usize;

    while t0.elapsed() < Duration::from_millis(MODBUS_TIMEOUT_MS as u64) {
        // Coba baca chunk kecil yang tersedia
        match uart.read(&mut buf[idx..]) {
            Ok(n) if n > 0 => {
                idx += n;
                last_byte = Instant::now();
                if expected_len > 0 && idx >= expected_len { break; }
                if idx >= buf.len() { break; }
            }
            _ => {
                // Tidak ada data baru: cek inter-byte gap (frame selesai)
                if idx > 0 && last_byte.elapsed() > Duration::from_millis(INTERBYTE_GAP_MS as u64) {
                    break;
                }
            }
        }
    }
    idx
}

// ======================= Core Modbus Read =======================
fn modbus_read_registers<P>(
    uart: &mut Uart<'_, esp_hal::Blocking>,
    mut de_pin: Option<&mut P>,      // <-- generik, bisa None (auto-direction)
    slave_id: u8,
    func: u8,
    start_addr: u16,
    count: u16,
    out_regs: &mut [u16],
) -> bool
where
    P: HalOutputPin,
{
    if count == 0 || count as usize > out_regs.len() || count > 10 { return false; }

    // Build request
    let mut req = [0u8; 8];
    req[0] = slave_id;
    req[1] = func;
    req[2] = (start_addr >> 8) as u8;
    req[3] = (start_addr & 0xFF) as u8;
    req[4] = (count >> 8) as u8;
    req[5] = (count & 0xFF) as u8;
    let crc = modbus_crc16(&req[..6]);
    req[6] = (crc & 0xFF) as u8;   // CRC low
    req[7] = (crc >> 8) as u8;     // CRC high

    clear_uart_rx(uart);

    // TX enable (jika manual)
    if let Some(p) = de_pin.as_mut() { let _ = p.set_high(); }
    let _ = uart_write_all(uart, &req);
    // silent interval sebelum balik ke RX
    sleep_ms(SILENT_INTERVAL_TX_MS);
    if let Some(p) = de_pin.as_mut() { let _ = p.set_low(); }

    // Baca respons
    let expected_len = 5usize + 2 * (count as usize);
    let mut resp = [0u8; 64];
    let got = read_response(uart, &mut resp, expected_len);
    if got < 5 || resp[0] != slave_id { return false; }

    // Exception?
    if resp[1] == (func | 0x80) {
        if got >= 3 { println!("Modbus exception code: {}", resp[2]); }
        return false;
    }
    if resp[1] != func { return false; }

    let byte_count = resp[2] as usize;
    if byte_count != 2 * (count as usize) { return false; }
    if got < 3 + byte_count + 2 { return false; }

    // CRC
    let crc_calc = modbus_crc16(&resp[..(3 + byte_count)]);
    let crc_resp = (resp[3 + byte_count] as u16) | ((resp[3 + byte_count + 1] as u16) << 8);
    if crc_calc != crc_resp {
        println!("CRC mismatch");
        return false;
    }

    // Data
    for i in 0..(count as usize) {
        let hi = resp[3 + 2*i] as u16;
        let lo = resp[3 + 2*i + 1] as u16;
        out_regs[i] = (hi << 8) | lo;
    }
    true
}

fn modbus_read_u16<P>(
    uart: &mut Uart<'_, esp_hal::Blocking>,
    de_pin: Option<&mut P>,          // <-- pass-through ke fungsi atas
    reg_addr: u16,
    out: &mut u16,
) -> bool
where
    P: HalOutputPin,
{
    let mut tmp = [0u16; 1];
    let ok = modbus_read_registers(uart, de_pin, MODBUS_SLAVE_ID, FUNCTION_CODE, reg_addr, 1, &mut tmp);
    if ok { *out = tmp[0]; }
    ok
}

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // --- UART1 di GPIO17 (TX) / GPIO18 (RX) ---
    let mut uart1 = Uart::new(
        peripherals.UART1,
        UartConfig::default().with_baudrate(MODBUS_BAUD), // default 8N1
    )
    .expect("UART1 init failed")            // <-- unwrap Result<Uart, ConfigError>
    .with_tx(peripherals.GPIO17)
    .with_rx(peripherals.GPIO18);

    // Jika perlu kontrol DE/RE manual, inisialisasi pin di sini.
    // Contoh (sesuaikan API versi esp-hal Anda):
    // use esp_hal::gpio::Output;
    // let mut de: Option<Output<_, esp_hal::gpio::PushPull>> = Some(peripherals.GPIOx.into_push_pull_output());
    let mut de: Option<Output<'_>> = None;

    println!("RAW Modbus ESP32-S3 start (UART1 TX=GPIO17, RX=GPIO18)");

    loop {
        let mut raw_h = 0u16;
        let mut raw_t = 0u16;

        let ok_h = modbus_read_u16(&mut uart1, de.as_mut().map(|p| p), REG_HUM, &mut raw_h);
        let ok_t = modbus_read_u16(&mut uart1, de.as_mut().map(|p| p), REG_TMP, &mut raw_t);

        if ok_h && ok_t {
            let rh = (raw_h as f32) / SCALE;
            // suhu bisa bertanda; gunakan interpretasi signed bila masuk akal
            let t_u = (raw_t as f32) / SCALE;
            let t_s = ((raw_t as i16) as f32) / SCALE;
            let t = if (-40.0..=125.0).contains(&t_s) { t_s } else { t_u };

            println!("RH: {:.1} %  |  T: {:.1} °C", rh, t);
        } else {
            println!("Gagal baca sebagian/semua register.");
        }

        sleep_ms(1000);
    }
}
