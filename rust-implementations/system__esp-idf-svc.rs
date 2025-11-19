// [package]
// name = "esp32_firebase_rest"
// version = "0.1.0"
// edition = "2021"

// [dependencies]
// # ESP-IDF Rust bindings
// esp-idf-sys = { version = "0.39", features = ["binstart"] }
// esp-idf-svc = "0.39"
// esp-idf-hal = "0.39"

// # For JSON handling
// serde = { version = "1.0", features = ["derive"] }
// serde_json = "1.0"

// # Convenience & error handling
// anyhow = "1.0"
// log = "0.4"
// esp-backtrace = "0.2"

// [profile.release]
// opt-level = "z"

// src/main.rs
use anyhow::{anyhow, Context, Result};
use esp_idf_hal::prelude::*;
use esp_idf_hal::gpio::PinDriver;
use esp_idf_svc::nvs::EspDefaultNvs;
use esp_idf_svc::netif::EspNetifStack;
use esp_idf_svc::sysloop::EspSystemEventLoop;
use esp_idf_svc::wifi::{EspWifi, ClientConfiguration};
use esp_idf_svc::http::client::{EspHttpClient, EspHttpClientConfiguration, Body, HttpConnection};
use esp_idf_sys as _; // initialize esp-idf bindings
use serde_json::Value;
use std::time::Duration;
use std::thread;
use log::{info, error};

// ===== CONFIG - replace these with your values =====
const WIFI_SSID: &str = "<YOUR_WIFI_SSID>";
const WIFI_PASS: &str = "<YOUR_WIFI_PASSWORD>";
const DATABASE_URL: &str = "https://andrew-5d2ad-default-rtdb.firebaseio.com"; // NO trailing slash here
// If you have a database secret or REST auth token, set it here (or set to None for public DB)
const FIREBASE_AUTH: Option<&str> = Some("<YOUR_FIREBASE_DB_SECRET_OR_AUTH_TOKEN>");

// Relay GPIOs (active low)
const RELAY_PINS: [u32; 4] = [18, 5, 17, 16]; // adjust if needed

const POLL_INTERVAL_MS: u64 = 2000; // 2 seconds (conservative)

// Helper to build firebase URL for a path, including .json and optional auth param
fn firebase_url(path: &str) -> String {
    if let Some(auth) = FIREBASE_AUTH {
        format!("{}/{}.json?auth={}", DATABASE_URL.trim_end_matches('/'), path.trim_start_matches('/'), auth)
    } else {
        format!("{}/{}.json", DATABASE_URL.trim_end_matches('/'), path.trim_start_matches('/'))
    }
}

fn make_device_id() -> String {
    // Use efuse MAC to generate ID like "ESP32_<HEX><HEX>"
    // esp_efuse_mac_get_default returns u64
    let mac = unsafe { esp_idf_sys::esp_efuse_mac_get_default() };
    format!("ESP32_{:X}{:X}", ((mac >> 32) as u32), (mac as u32)).to_ascii_uppercase()
}

fn main() -> Result<()> {
    // Initialize logger/backtrace + ESP-IDF
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("Starting esp32_firebase_rest POC");

    // Bring up system services
    let sysloop = EspSystemEventLoop::take().context("take sysloop")?;
    let netif = EspNetifStack::new().context("create netif")?;
    let _nvs = EspDefaultNvs::new().context("create nvs")?;

    // WIFI
    let mut wifi = EspWifi::new(netif.clone(), sysloop.clone(), Some(_nvs)).context("create wifi")?;
    wifi.set_configuration(&ClientConfiguration {
        ssid: WIFI_SSID.into(),
        password: WIFI_PASS.into(),
        ..Default::default()
    }).context("set wifi config")?;

    info!("Connecting to Wi-Fi...");
    wifi.start().context("wifi start")?;
    wifi.connect().context("wifi connect")?;

    // Wait for connection (with timeout)
    let mut connected = false;
    for _ in 0..40 {
        if wifi.is_connected().unwrap_or(false) {
            connected = true;
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    if !connected {
        return Err(anyhow!("Failed to connect to Wi-Fi"));
    }
    info!("Wi-Fi connected: {}", wifi.sta_netif().get_ip_info().unwrap().ip);

    // Device ID
    let device_id = make_device_id();
    info!("Device ID: {}", device_id);

    // Create HTTP client configuration (default is ok; adjust timeouts if needed)
    let http_conf = EspHttpClientConfiguration {
        // You may adjust timeout, buffer sizes, and certificates here if advanced TLS handling is needed
        ..Default::default()
    };
    let mut client = EspHttpClient::new(&http_conf).context("create http client")?;

    // Initialize relays: configure GPIO outputs and set HIGH (OFF) because active-low
    let peripherals = Peripherals::take().context("Peripherals::take")?;
    let pins = peripherals.pins;
    let mut relay_drivers: Vec<PinDriver<_, Output>> = Vec::new();
    // Map pin numbers to pin objects — using esp-idf-hal high-level pins mapping
    // Note: Pin names vary by target board. We assume generic pin mapping via `pins.gpioX`.
    for &gpio in RELAY_PINS.iter() {
        // convert number to pin reference — esp-idf-hal exposes pins as fields (gpio0..)
        // For brevity, use match for the four pins used. Extend as needed.
        let driver = match gpio {
            18 => PinDriver::output(pins.gpio18).context("gpio18")?,
            5 => PinDriver::output(pins.gpio5).context("gpio5")?,
            17 => PinDriver::output(pins.gpio17).context("gpio17")?,
            16 => PinDriver::output(pins.gpio16).context("gpio16")?,
            other => return Err(anyhow!("Unsupported gpio: {}", other)),
        };
        // set HIGH (OFF)
        driver.set_high().context("set relay high")?;
        relay_drivers.push(driver);
    }

    // Initialize Firebase DB entries for this device
    let base_path = format!("devices/{}", device_id);
    let status_url = firebase_url(&format!("{}/status", base_path));
    info!("Setting status -> online: {}", status_url);

    // PUT status = "online"
    {
        let body = Body::Bytes(br#""online""#.to_vec()); // JSON string literal
        let mut req = client.put(&status_url)?;
        req.header("Content-Type", "application/json")?;
        req.body(body)?;
        let _ = req.submit()?;
    }

    // Initialize relays to 0
    for name in &["relay1","relay2","relay3","relay4"] {
        let p = firebase_url(&format!("{}/relays/{}", base_path, name));
        let body = Body::Bytes(b"0".to_vec()); // JSON number 0
        let mut req = client.put(&p)?;
        req.header("Content-Type", "application/json")?;
        req.body(body)?;
        let _ = req.submit()?;
        info!("Initialized {}", p);
    }

    info!("Initialization complete. Entering poll loop.");

    // Poll loop
    loop {
        // keep WiFi alive — if disconnected, attempt reconnect (non-blocking)
        if !wifi.is_connected().unwrap_or(false) {
            error!("Wi-Fi disconnected; attempting reconnect");
            // try reconnecting once, non-blocking
            let _ = wifi.disconnect();
            let _ = wifi.connect();
            thread::sleep(Duration::from_millis(500));
        }

        // Poll each relay
        for (i, name) in ["relay1","relay2","relay3","relay4"].iter().enumerate() {
            let path = format!("devices/{}/relays/{}", device_id, name);
            let url = firebase_url(&path);

            // GET request
            match client.get(&url) {
                Ok(mut req) => {
                    // submit and read response body
                    match req.submit() {
                        Ok(mut resp) => {
                            let mut buf: Vec<u8> = Vec::new();
                            // read response into buffer (stream)
                            loop {
                                let mut chunk = [0u8; 512];
                                match resp.read(&mut chunk) {
                                    Ok(0) => break, // EOF
                                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                                    Err(e) => {
                                        error!("Error reading response: {:?}", e);
                                        break;
                                    }
                                }
                            }
                            // Parse JSON value — Firebase returns e.g. 0 or 1
                            if let Ok(text) = String::from_utf8(buf.clone()) {
                                // text might contain whitespace/newlines
                                let trimmed = text.trim();
                                // Attempt parse as integer
                                match serde_json::from_str::<Value>(trimmed) {
                                    Ok(val) => {
                                        // expect number 0 or 1
                                        if let Some(n) = val.as_i64() {
                                            let desired = n as i32;
                                            let should_on = desired == 1;
                                            // active low: LOW -> ON (0), HIGH -> OFF (1)
                                            let driver = &mut relay_drivers[i];
                                            if should_on {
                                                let _ = driver.set_low();
                                            } else {
                                                let _ = driver.set_high();
                                            }
                                            info!("{} -> {}", name, if should_on { "ON" } else { "OFF" });
                                        } else {
                                            error!("Unexpected value for {}: {:?}", name, val);
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed parse JSON for {}: {} / raw: `{}`", name, e, trimmed);
                                    }
                                }
                            } else {
                                error!("Response not UTF-8 for {}", name);
                            }
                        }
                        Err(e) => {
                            error!("HTTP submit error for {}: {:?}", name, e);
                        }
                    }
                }
                Err(e) => {
                    error!("HTTP GET request creation failed for {}: {:?}", name, e);
                }
            }
        }

        // Sleep between polls
        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
}

