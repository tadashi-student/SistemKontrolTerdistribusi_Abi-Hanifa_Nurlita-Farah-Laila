import serial
import time
import re
from influxdb_client import InfluxDBClient, Point, WriteOptions

# --- PENGATURAN - SILAKAN SESUAIKAN ---

# Pengaturan Serial Port
# Pastikan ini adalah port COM yang benar untuk ESP32 Anda
ESP32_PORT = 'COM12' 

INFLUX_URL = "http://localhost:8086"
INFLUX_ORG = "ITS"  
INFLUX_BUCKET = "SKT" 


INFLUX_TOKEN = "LvJKbsRaVoniO0htQf9p4jQSu221aN89dTXL6EI4HN-0PY-Q3a3gsQiijO8RHDzu6I6Q_63_sUuA41ZX3XOmxg==" # <-- GANTI INI

# --- KODE UTAMA ---

try:
    # Siapkan koneksi ke InfluxDB
    influx_client = InfluxDBClient(url=INFLUX_URL, token=INFLUX_TOKEN, org=INFLUX_ORG)
    write_api = influx_client.write_api(write_options=WriteOptions(batch_size=1))
    print("Koneksi ke InfluxDB disiapkan.")

    # Buka koneksi serial ke ESP32
    ser = serial.Serial(ESP32_PORT, 115200, timeout=2)
    print(f"Berhasil terhubung ke ESP32 di port {ESP32_PORT}")

    temp_nyata = None
    rh_nyata = None

    while True:
        line = ser.readline().decode('utf-8').strip()
        
        if line:
            match_temp = re.search(r"T\s*=\s*([0-9]+\.[0-9]+)\s*°C", line)
            match_rh = re.search(r"RH\s*=\s*([0-9]+\.[0-9]+)\s*%", line)

            if match_temp:
                temp_nyata = float(match_temp.group(1))
            
            if match_rh:
                rh_nyata = float(match_rh.group(1))

            if temp_nyata is not None and rh_nyata is not None:
                print(f"Data diterima -> Suhu: {temp_nyata}°C, Kelembapan: {rh_nyata}%")
# Kode BARU
                point = Point("data_sht20") \
                    .field("temperature", temp_nyata) \
                    .field("humidity", rh_nyata)

                write_api.write(bucket=INFLUX_BUCKET, org=INFLUX_ORG, record=point)
                print("--> Data berhasil dikirim ke InfluxDB.")
                
                temp_nyata = None
                rh_nyata = None

except serial.SerialException:
    print(f"Error: Tidak bisa membuka port {ESP32_PORT}. Pastikan ESP32 terhubung dan tidak ada program monitor lain yang berjalan.")
except KeyboardInterrupt:
    print("\nProgram dihentikan.")
finally:
    if 'ser' in locals() and ser.is_open:
        ser.close()
        print("Koneksi serial ditutup.")
    if 'influx_client' in locals():
        influx_client.close()
        print("Koneksi InfluxDB ditutup.")