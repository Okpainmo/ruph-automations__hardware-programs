// [package]
// name = "esp32-firebase-relays"
// version = "0.1.0"
// edition = "2021"

// [dependencies]
// esp-idf-sys = { version = "0.34", features = ["binstart"] }
// esp-idf-hal = "0.43"
// esp-idf-svc = "0.46"
// embedded-svc = "0.25"
// firebase-rs = "0.5"
// serde = { version = "1", features = ["derive"] }
// serde_json = "1"

// [build-dependencies]
// embuild = "0.31"


use esp_idf_hal::gpio::*;
use esp_idf_hal::prelude::*;
use esp_idf_svc::wifi::*;
use embedded_svc::wifi::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use firebase_rs::Firebase;
use std::thread;

// ============================
// CONFIG
// ============================

const WIFI_SSID: &str = "Infinix NOTE 30 VIP";
const WIFI_PASSWORD: &str = "12345678p";
const DATABASE_URL: &str =
    "https://andrew-5d2ad-default-rtdb.firebaseio.com";

const RELAY_PATHS: [&str; 4] = ["relay1", "relay2", "relay3", "relay4"];
const POLL_INTERVAL_MS: u64 = 1000;

// ============================
// FIREBASE STRUCT
// ============================

#[derive(Debug, Serialize, Deserialize)]
struct RelayValue {
    value: i32,
}

// ============================
// MAIN ENTRY
// ============================

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();

    // ---------------------------
    // GPIO SETUP (ACTIVE LOW)
    // ---------------------------
    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;

    let mut relays: [PinDriver<_, Output>; 4] = [
        PinDriver::output(pins.gpio18)?, // RELAY1
        PinDriver::output(pins.gpio5)?,  // RELAY2
        PinDriver::output(pins.gpio17)?, // RELAY3
        PinDriver::output(pins.gpio16)?, // RELAY4
    ];

    // Start all relays OFF (HIGH = off, because ACTIVE LOW)
    for r in relays.iter_mut() {
        r.set_high()?;
    }

    // ---------------------------
    // CONNECT WIFI
    // ---------------------------
    let mut wifi = EspWifi::new(
        peripherals.modem,
        EspSystemEventLoop::take()?,
        Some(EspDefaultNvs::new()?),
    )?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID.into(),
        password: WIFI_PASSWORD.into(),
        ..Default::default()
    }))?;

    wifi.start()?;
    wifi.connect()?;

    println!("[WiFi] Connecting...");
    while !wifi.is_connected().unwrap_or(false) {
        thread::sleep(Duration::from_millis(300));
    }
    println!("[WiFi] Connected!");

    // ---------------------------
    // DEVICE ID
    // ---------------------------
    let mac = wifi.sta_netif().get_mac()?;
    let device_id = format!(
        "ESP32_{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    println!("[DeviceID] {}", device_id);

    // ---------------------------
    // FIREBASE CLIENT
    // ---------------------------
    let fb_root = format!("{}/devices/{}", DATABASE_URL, device_id);
    let firebase = Firebase::new(&fb_root)?;

    // Mark device online
    firebase.at("status").set(&serde_json::json!("online"))?;

    // Initialize relay values to 0
    for name in RELAY_PATHS {
        firebase
            .at(&format!("relays/{}", name))
            .set(&serde_json::json!(0))?;
    }

    println!("[System] Ready. Polling...");

    // ============================
    // MAIN LOOP
    // ============================
    loop {
        // Poll Firebase
        for (index, relay_name) in RELAY_PATHS.iter().enumerate() {
            let fb_path = format!("relays/{}", relay_name);

            let result = firebase.at(&fb_path).get::<i32>();

            let desired = match result {
                Ok(value) => value,
                Err(e) => {
                    println!("[Firebase] Read error for {}: {:?}", relay_name, e);
                    continue;
                }
            };

            // ACTIVE LOW logic: 1 = ON → LOW, 0 = OFF → HIGH
            let should_on = desired == 1;

            if should_on {
                relays[index].set_low()?;
            } else {
                relays[index].set_high()?;
            }

            println!(
                "[Relay] {} → {}",
                relay_name,
                if should_on { "ON" } else { "OFF" }
            );
        }

        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
}
