# bridge_csv_to_mqtt.py
import time, csv, os
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler
import paho.mqtt.client as mqtt
import json

CSV_PATH = r"C:\temp\sim_out.csv"
MQTT_HOST, MQTT_PORT, TOPIC = "127.0.0.1", 1883, "dwsim/sim1"

cli = mqtt.Client("csv-bridge")
cli.connect(MQTT_HOST, MQTT_PORT, 60)

def publish_last_row():
    if not os.path.exists(CSV_PATH):
        return
    with open(CSV_PATH, newline="") as f:
        last = None
        for row in csv.DictReader(f):
            last = row
    if not last:
        return
    payload = {
        "ts": int(last["ts"]),
        "sim": {
            "feed": {
                "T_K":   float(last["feed_T_K"]),
                "P_kPa": float(last["feed_P_kPa"]),
                "F_kg_s":float(last["feed_F_kg_s"])
            },
            "product": {
                "T_K":   float(last["prod_T_K"]),
                "P_kPa": float(last["prod_P_kPa"]),
                "F_kg_s":float(last["prod_F_kg_s"])
            }
        }
    }
    cli.publish(TOPIC, json.dumps(payload))

class Handler(FileSystemEventHandler):
    def on_modified(self, event):
        if event.src_path.lower() == CSV_PATH.lower():
            publish_last_row()

if __name__ == "__main__":
    # publish sekali saat start (kalau file sudah ada)
    publish_last_row()

    # pantau perubahan file
    obs = Observer()
    obs.schedule(Handler(), os.path.dirname(CSV_PATH) or ".", recursive=False)
    obs.start()
    try:
        while True:
            cli.loop(0.1)
            time.sleep(0.2)
    except KeyboardInterrupt:
        obs.stop()
    obs.join()
