import os, time, csv, threading, re
from pathlib import Path

# ============ KONFIG ============
CSV_PATH      = r"C:\DCS\sim_out.csv"   # harus sama dengan di DWSIM IronPython
ESP32_PORT    = "COM12"                 # ganti sesuai port ESP32 S3
ESP32_BAUD    = 115200

# --- InfluxDB konfigurasi kamu ---
INFLUX_URL    = "http://localhost:8086"
INFLUX_ORG    = "ITS"
INFLUX_BUCKET = "SKT"
INFLUX_TOKEN  = "LvJKbsRaVoniO0htQf9p4jQSu221aN89dTXL6EI4HN-0PY-Q3a3gsQiijO8RHDzu6I6Q_63_sUuA41ZX3XOmxg=="
# ================================

# ---------- Serial reader (ambil suhu & RH dari ESP32) ----------
import serial
last_temp = None
last_rh   = None
lock = threading.Lock()

def serial_reader():
    global last_temp, last_rh
    try:
        ser = serial.Serial(ESP32_PORT, ESP32_BAUD, timeout=2)
        print(f"[SERIAL] connected {ESP32_PORT} @ {ESP32_BAUD}")
    except Exception as e:
        print("[SERIAL] gagal buka port:", e)
        return

    pat_t  = re.compile(r"T\s*=\s*([+-]?\d+(?:\.\d+)?)\s*(?:°?\s*C)?", re.IGNORECASE)
    pat_rh = re.compile(r"RH\s*=\s*([+-]?\d+(?:\.\d+)?)\s*%?", re.IGNORECASE)

    while True:
        try:
            line = ser.readline().decode("utf-8", errors="ignore").strip()
            if not line:
                continue
            m1 = pat_t.search(line)
            m2 = pat_rh.search(line)
            with lock:
                if m1: last_temp = float(m1.group(1))
                if m2: last_rh   = float(m2.group(1))
        except Exception as e:
            print("[SERIAL] error:", e)
            time.sleep(1)

# ---------- Influx writer ----------
from influxdb_client import InfluxDBClient, Point, WriteOptions
influx = InfluxDBClient(url=INFLUX_URL, token=INFLUX_TOKEN, org=INFLUX_ORG)
write_api = influx.write_api(write_options=WriteOptions(batch_size=1))

def write_join_to_influx(row):
    with lock:
        t = last_temp
        h = last_rh
    if t is None or h is None:
        return  # tunggu sampai data sensor ada

    # CSV kolom dari DWSIM
    feed_Tc = float(row["feed_T_K"]) - 273.15
    feed_Pk = float(row["feed_P_kPa"])
    feed_F  = float(row["feed_F_kg_s"])
    prod_Tc = float(row["prod_T_K"]) - 273.15
    prod_Pk = float(row["prod_P_kPa"])
    prod_F  = float(row["prod_F_kg_s"])
    ts_ms   = int(row["ts"])  # milidetik

    p = (
        Point("process_join")
        .field("sens_temp_c", float(t))
        .field("sens_rh_pct", float(h))
        .field("feed_t_c", feed_Tc)
        .field("feed_p_kPa", feed_Pk)
        .field("feed_f_kg_s", feed_F)
        .field("prod_t_c", prod_Tc)
        .field("prod_p_kPa", prod_Pk)
        .field("prod_f_kg_s", prod_F)
        .time(ts_ms, write_precision="ms")
    )
    write_api.write(bucket=INFLUX_BUCKET, org=INFLUX_ORG, record=p)
    print(f"[JOIN→Influx] sens T={t:.2f} RH={h:.2f} | feed Tc={feed_Tc:.2f} P={feed_Pk:.2f} | prod Tc={prod_Tc:.2f} P={prod_Pk:.2f}")

# ---------- CSV reader ----------
def read_last_row(retry=5, delay=0.05):
    if not os.path.exists(CSV_PATH):
        return None
    for _ in range(retry):
        try:
            last = None
            with open(CSV_PATH, newline="") as f:
                for row in csv.DictReader(f):
                    last = row
            return last
        except Exception:
            time.sleep(delay)
    return None

# ---------- Watch CSV (modified & created) + polling fallback ----------
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler
import os

CSV_PATH_NORM = os.path.normcase(os.path.abspath(CSV_PATH))
def _is_csv(path):
    try:
        return os.path.normcase(os.path.abspath(path)) == CSV_PATH_NORM
    except Exception:
        return False

class Handler(FileSystemEventHandler):
    def on_created(self, event):
        if not event.is_directory and _is_csv(event.src_path):
            row = read_last_row()
            if row: write_join_to_influx(row)
    def on_modified(self, event):
        if not event.is_directory and _is_csv(event.src_path):
            row = read_last_row()
            if row: write_join_to_influx(row)

def poll_tail(interval=1.0):
    last_ts = None
    while True:
        try:
            r = read_last_row()
            if r:
                ts = r.get("ts")
                if ts and ts != last_ts:
                    write_join_to_influx(r)
                    last_ts = ts
        except Exception as e:
            print("[POLL] error:", e)
        time.sleep(interval)

# ---------- MAIN ----------
if __name__ == "__main__":
    threading.Thread(target=serial_reader, daemon=True).start()

    folder = os.path.dirname(CSV_PATH) or "."
    os.makedirs(folder, exist_ok=True)

    observer = Observer()
    observer.schedule(Handler(), folder, recursive=False)
    observer.start()

    threading.Thread(target=poll_tail, args=(1.0,), daemon=True).start()

    # kirim satu kali kalau file sudah ada
    r = read_last_row()
    if r: write_join_to_influx(r)

    print("[READY] Watching CSV & streaming ke Influx. Pastikan script IronPython DWSIM sudah dijalankan.")
    try:
        while True:
            time.sleep(0.2)
    except KeyboardInterrupt:
        pass
    finally:
        observer.stop()
        observer.join()
        influx.close()
