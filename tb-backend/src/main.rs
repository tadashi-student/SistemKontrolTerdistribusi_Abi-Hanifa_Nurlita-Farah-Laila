use anyhow::{Result};
use dotenvy::dotenv;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use rumqttc::{AsyncClient, Event, MqttOptions, QoS};
use serde::Deserialize;
use std::{env, time::Duration};
use tokio::time::sleep;

#[derive(Debug, Deserialize, Default)]
struct Row {
    #[serde(rename = "_time", default)] _time: Option<String>,
    #[serde(default)] sens_temp_c: Option<f64>,
    #[serde(default)] sens_rh_pct: Option<f64>,
    #[serde(default)] feed_t_c: Option<f64>,
    #[serde(default)] feed_p_kPa: Option<f64>,
    #[serde(default)] feed_f_kg_s: Option<f64>,
    #[serde(default)] prod_t_c: Option<f64>,
    #[serde(default)] prod_p_kPa: Option<f64>,
    #[serde(default)] prod_f_kg_s: Option<f64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();

    // --- Load ENV (runtime) ----
    let influx_url    = env::var("INFLUX_URL")?;
    let influx_org    = env::var("INFLUX_ORG")?;
    let influx_bucket = env::var("INFLUX_BUCKET")?;
    let influx_token  = env::var("INFLUX_TOKEN")?;

    let tb_host  = env::var("TB_HOST")?;
    let tb_port  = env::var("TB_PORT").unwrap_or_else(|_| "1883".into()).parse::<u16>()?;
    let tb_token = env::var("TB_TOKEN")?;
    let interval = env::var("PUSH_INTERVAL").unwrap_or_else(|_| "1".into()).parse::<u64>()?;

    // --- MQTT: AsyncClient (ThingsBoard) ---
    let mut opts = MqttOptions::new("rust-backend", tb_host, tb_port);
    // TB: username = device token, password kosong
    opts.set_credentials(tb_token, "");
    opts.set_keep_alive(Duration::from_secs(30));

    let (mqtt, mut eventloop) = AsyncClient::new(opts, 10);
    tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(_)) => { /* no-op */ }
                Ok(Event::Outgoing(_)) => { /* no-op */ }
                Err(e) => {
                    eprintln!("[MQTT] eventloop error: {e}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });

    let http = reqwest::Client::new();

    loop {
        match pull_last_from_influx_csv(&http, &influx_url, &influx_org, &influx_bucket, &influx_token).await {
            Ok(Some(row)) => {
                let payload = build_tb_payload(&row);
                let json = serde_json::to_string(&payload)?;
                // TB telemetry topic
                mqtt.publish("v1/devices/me/telemetry", QoS::AtLeastOnce, false, json)
                    .await?;
                println!("[TB] published: {}", payload_summary(&row));
            }
            Ok(None) => eprintln!("[Influx] no data (process_join) in last window."),
            Err(e) => eprintln!("[Influx] error: {e:#}"),
        }
        sleep(Duration::from_secs(interval)).await;
    }
}

// Query InfluxDB v2 (CSV) dan ambil 1 baris pivot terakhir
async fn pull_last_from_influx_csv(
    http: &reqwest::Client,
    url: &str,
    org: &str,
    bucket: &str,
    token: &str,
) -> Result<Option<Row>> {
    let flux = format!(r#"
from(bucket: "{bucket}")
  |> range(start: -15m)
  |> filter(fn: (r) => r._measurement == "process_join")
  |> last()
  |> pivot(rowKey:["_time"], columnKey: ["_field"], valueColumn: "_value")
"#);

    let endpoint = format!(
        "{}/api/v2/query?org={}",
        url.trim_end_matches('/'),
        urlencoding::encode(org)
    );

    let resp = http
        .post(&endpoint)
        .header(AUTHORIZATION, format!("Token {}", token))
        .header(ACCEPT, "application/csv")
        .header(CONTENT_TYPE, "application/vnd.flux")
        .body(flux)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .has_headers(true)
        .from_reader(resp.as_bytes());

    for result in rdr.deserialize::<Row>() {
        let row: Row = result?;
        if row._time.is_some() {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

fn build_tb_payload(row: &Row) -> serde_json::Value {
    serde_json::json!({
        "sens_temp_c": row.sens_temp_c.unwrap_or_default(),
        "sens_rh_pct": row.sens_rh_pct.unwrap_or_default(),
        "feed_t_c":    row.feed_t_c.unwrap_or_default(),
        "feed_p_kPa":  row.feed_p_kPa.unwrap_or_default(),
        "feed_f_kg_s": row.feed_f_kg_s.unwrap_or_default(),
        "prod_t_c":    row.prod_t_c.unwrap_or_default(),
        "prod_p_kPa":  row.prod_p_kPa.unwrap_or_default(),
        "prod_f_kg_s": row.prod_f_kg_s.unwrap_or_default()
    })
}

fn payload_summary(r: &Row) -> String {
    format!(
        "sens(T={:.2}°C,RH={:.2}%), feed(T={:.2}°C,P={:.2} kPa), prod(T={:.2}°C,P={:.2} kPa)",
        r.sens_temp_c.unwrap_or(0.0),
        r.sens_rh_pct.unwrap_or(0.0),
        r.feed_t_c.unwrap_or(0.0),
        r.feed_p_kPa.unwrap_or(0.0),
        r.prod_t_c.unwrap_or(0.0),
        r.prod_p_kPa.unwrap_or(0.0)
    )
}
