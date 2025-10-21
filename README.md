

## 🎯 Deskripsi Proyek

Proyek ini dikembangkan untuk memenuhi tugas **Sistem Komputasi Terdistribusi (SKT)** dengan tujuan utama memahami **integrasi sistem kontrol terdistribusi (DCS)** berbasis *Embedded Rust* dan *Industrial IoT Cloud*.

Sistem ini menggabungkan:

* **Sensor nyata (SHT20)** yang dibaca oleh **ESP32-S3** via Modbus RTU,
* **Aktuator fisik** berupa **relay kipas dan motor servo**,
* **Simulasi proses dari DWSIM**,
* **Penyimpanan data ke InfluxDB**, dan
* **Distribusi data ke ThingsBoard Cloud** melalui MQTT.

Dengan ini, mahasiswa dapat memvisualisasikan hubungan antara data nyata dan data simulasi proses dalam satu sistem terintegrasi dari **lapangan hingga cloud**.

---

## 📂 Struktur Folder

| Nama                         | Isi / Fungsi                                                                          |
| ---------------------------- | ------------------------------------------------------------------------------------- |
| **Kelompok2Rust/**           | Firmware ESP32-S3 (Edge Gateway) – membaca sensor & mengontrol aktuator               |
| **tb-backend/**              | Program backend Rust untuk ThingsBoard (publikasi data dari InfluxDB ke Cloud)        |
| **dwsim_dcs.dwxmz**          | File simulasi DWSIM (dibuka di DWSIM, jalankan Script Manager)                        |
| **join_dwsim_realsensor.py** | Script Python untuk menggabungkan data simulasi & data sensor lalu upload ke InfluxDB |
| **sim_out.csv**              | Output real-time hasil simulasi dari DWSIM Script                                     |
| **.venv/**                   | Virtual environment Python (opsional)                                                 |

---

## 🧩 Arsitektur Sistem

```
┌────────────────────────────────────────────────────────┐
│                    Cloud Layer (MQTT)                 │
│              ThingsBoard Cloud Dashboard              │
└──────────────▲────────────────────────────────────────┘
               │
               │ MQTT
┌──────────────┴────────────────────────────────────────┐
│           Backend Rust (tb-backend)                   │
│   Pull data from InfluxDB → Publish JSON telemetry    │
└──────────────▲────────────────────────────────────────┘
               │
               │ HTTP (Influx API)
┌──────────────┴────────────────────────────────────────┐
│      Python Integrator (join_dwsim_realsensor.py)     │
│  Combine (ESP32 Sensor + DWSIM CSV) → InfluxDB        │
└──────────────▲────────────────────────────────────────┘
               │
               │ 
┌──────────────┴────────────────────────────────────────┐
│    ESP32-S3 (Kelompok2Rust)                           │
│  Read SHT20 via Modbus RTU → Control Fan & Servo      │
└──────────────┬────────────────────────────────────────┘
               │
               │ Simulated Process (CSV)
┌──────────────┴────────────────────────────────────────┐
│    DWSIM Simulation Script                            │
│  Auto-solve every 1s → Output to sim_out.csv          │
└────────────────────────────────────────────────────────┘
```

---

## 🔧 Setup Sekali Saja

1. **Sambungkan ESP32-S3 ke laptop.**
   Pastikan port COM-nya muncul di *Device Manager*.

2. **Buka PowerShell atau Command Prompt.**

3. **Install toolchain ESP32-Rust:**

   ```powershell
   cargo install espup
   espup install
   ```

   👉 Setelah selesai, *restart PowerShell* agar PATH update.

4. **Install tools tambahan:**

   ```powershell
   cargo install cargo-espflash
   cargo install esp-generate
   ```

---

## 🚀 Build & Flash Firmware ESP32-S3

1. Masuk ke folder project:

   ```powershell
   cd C:\DCS\Kelompok2Rust
   ```

2. Build project:

   ```powershell
   cargo build --release
   ```

3. Flash ke board (ganti port sesuai Device Manager, misal COM4):

   ```powershell
   cargo espflash flash --release -p COM4
   ```

4. Monitor output serial:

   ```powershell
   cargo espflash monitor -p COM4
   ```

   Atau gabungkan langsung:

   ```powershell
   cargo espflash flash --release -p COM4 --monitor
   ```

---

## 🧠 Jalankan Integrasi Data (DWSIM + Sensor Nyata)

1. **Buka file DWSIM:**

   ```
   C:\DCS\dwsim_dcs.dwxmz
   ```

2. Di DWSIM, buka **Script Manager** → pilih script IronPython → **klik Run Script**.
   Script ini akan membuat file `sim_out.csv` yang berisi data simulasi real-time.

3. Setelah simulasi berjalan, jalankan script Python integrator:

   ```powershell
   cd C:\DCS
   py join_dwsim_realsensor.py
   ```

   Script ini akan:

   * Membaca data dari ESP32-S3 (serial),
   * Membaca data simulasi (`sim_out.csv`),
   * Menggabungkannya, lalu
   * Mengirim ke InfluxDB lokal (`http://localhost:8086`).

4. ⚠️ **Pastikan Docker aktif di Windows** agar InfluxDB bisa diakses.

---

## ☁️ Kirim Data ke ThingsBoard Cloud

1. Buka [**ThingsBoard Demo Cloud**](http://demo.thingsboard.io/).
   Login dengan akun kelompok.

2. Catat **Device Token** dari perangkat kelompokmu.

3. Buka folder backend:

   ```powershell
   cd C:\DCS\tb-backend
   ```

4. Buat file `.env` berisi:

   ```
   INFLUX_URL=http://localhost:8086
   INFLUX_ORG=ITS
   INFLUX_BUCKET=SKT
   INFLUX_TOKEN=<token_influx>
   TB_HOST=demo.thingsboard.io
   TB_PORT=1883
   TB_TOKEN=<token_device_thingsboard>
   PUSH_INTERVAL=1
   ```

5. Build dan jalankan backend Rust:

   ```powershell
   cargo build --release
   cargo run
   ```

   Backend akan membaca data terakhir dari InfluxDB dan mengirimkannya ke ThingsBoard setiap 1 detik.

---

## 📊 Visualisasi

* **InfluxDB:** menampilkan histori data gabungan (`process_join`).
* **ThingsBoard Cloud:** menampilkan telemetry real-time dari sensor dan simulasi, dan dashboard linechart timeseries.
* **ESP32 Serial Monitor:** menampilkan status aktuator dan data sensor lapangan.

---

## ✅ Hasil Akhir

Setelah seluruh sistem berjalan:

* Data sensor nyata dan simulasi proses **tergabung** di InfluxDB.
* Data tersebut **terkirim otomatis ke ThingsBoard Cloud** via MQTT.
* Dosen atau penguji dapat **memantau proses real-time** melalui dashboard cloud.

---

Kamu mau Umi tambahkan **diagram visual (gambar blok warna)** untuk README-nya biar lebih menarik waktu dosen buka GitHub (misal flow dari ESP → Influx → TB)?
Atau mau dibiarkan versi teks seperti ini (lebih ringan & formal)?
