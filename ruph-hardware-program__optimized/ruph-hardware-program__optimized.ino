#include <WiFi.h>
#include <Firebase_ESP_Client.h>
#include "addons/TokenHelper.h"
#include "addons/RTDBHelper.h"

// ====== Config ======
#define WIFI_SSID       "Infinix NOTE 30 VIP"
#define WIFI_PASSWORD   "12345678p"
#define API_KEY         "AIzaSyAkJcFWMqWUMbkKidbnd_lYCjhLEv5AiGI"
// NOTE: remove trailing slash to avoid path issues
#define DATABASE_URL    "https://andrew-5d2ad-default-rtdb.firebaseio.com"

#define RELAY4 16
#define RELAY3 17
#define RELAY2 5
#define RELAY1 18
#define STATUS_LED 2

// Relay pins array (active LOW)
const int relayPins[4] = { RELAY1, RELAY2, RELAY3, RELAY4 };
const char* relayNames[4] = { "relay1", "relay2", "relay3", "relay4" };

FirebaseData fbdo;
FirebaseAuth auth;
FirebaseConfig config;
String deviceID;

// Polling interval
unsigned long lastPollTime = 0;
const unsigned long POLL_INTERVAL = 2000;  // 2 seconds

// Optional: token callback is defined in addons/TokenHelper.h
// makes Firebase sign-in/debug visible
// config.token_status_callback = tokenStatusCallback;

void ensureWiFiSafe() {
  if (WiFi.status() == WL_CONNECTED) return;

  Serial.println("[WiFi] Lost connection. Reconnecting...");
  WiFi.disconnect();
  WiFi.begin(WIFI_SSID, WIFI_PASSWORD);

  unsigned long start = millis();
  // try for up to 10s
  while (WiFi.status() != WL_CONNECTED && millis() - start < 10000) {
    Serial.print(".");
    delay(300);
  }

  if (WiFi.status() == WL_CONNECTED) {
    Serial.println("\n[WiFi] Reconnected.");
  } else {
    Serial.println("\n[WiFi] Reconnect failed (will retry later).");
  }
}

void ensureFirebaseSafe() {
  // Don't repeatedly call Firebase.begin(); instead rely on Firebase.ready()
  if (Firebase.ready()) return;

  Serial.println("[Firebase] Not ready. Calling reconnectWiFi(true) to allow recovery.");
  Firebase.reconnectWiFi(true);
  delay(200); // allow the client to settle
}

void setup() {
  Serial.begin(115200);
  delay(100);

  // --- Wi-Fi ---
  WiFi.begin(WIFI_SSID, WIFI_PASSWORD);
  Serial.print(F("[WiFi] Connecting"));
  unsigned long wstart = millis();
  while (WiFi.status() != WL_CONNECTED && millis() - wstart < 15000) {
    Serial.print(F("."));
    delay(300);
  }
  if (WiFi.status() == WL_CONNECTED) {
    Serial.println(F("\n[WiFi] Connected!"));
    Serial.printf("[WiFi] IP: %s\n", WiFi.localIP().toString().c_str());
  } else {
    Serial.println(F("\n[WiFi] Failed to connect within timeout."));
    // proceed anyway — ensureWiFiSafe() will attempt later reconnects
  }

  // --- Device ID ---
  uint64_t chipid = ESP.getEfuseMac();
  deviceID = "ESP32_" + String((uint32_t)(chipid >> 32), HEX) +
             String((uint32_t)(chipid & 0xFFFFFFFF), HEX);
  deviceID.toUpperCase();
  Serial.println("[Device] ID: " + deviceID);

  // --- Firebase config ---
  config.api_key = API_KEY;
  config.database_url = DATABASE_URL;
  // helpful debug callback from TokenHelper
  config.token_status_callback = tokenStatusCallback;

  // Anonymous sign-up (check result)
  if (Firebase.signUp(&config, &auth, "", "")) {
    Serial.println("[Firebase] Anonymous sign-up OK");
  } else {
    Serial.printf("[Firebase] Sign-up failed: %s\n", config.signer.signupError.message.c_str());
  }

  Firebase.begin(&config, &auth);
  Firebase.reconnectWiFi(true);
  fbdo.setResponseSize(4096);

  // --- Relays (ACTIVE LOW) ---
  // Standardize: OFF = 0 in DB => Pin HIGH (inactive). ON = 1 => Pin LOW (active)
  for (int i = 0; i < 4; i++) {
    pinMode(relayPins[i], OUTPUT);
    digitalWrite(relayPins[i], HIGH); // OFF initially (active LOW)
  }

  // --- Firebase DB initialization ---
  // Wait for Firebase to be ready before writing initial states
  unsigned long startWait = millis();
  while (!Firebase.ready() && millis() - startWait < 5000) {
    Serial.print(".");
    delay(200);
  }
  Serial.println();

  if (!Firebase.ready()) {
    Serial.println("[Firebase] Not ready after startup timeout; continuing but DB writes may fail.");
  } else {
    String basePath = "/devices/" + deviceID;
    // Set status
    if (!Firebase.RTDB.setString(&fbdo, basePath + "/status", "online")) {
      Serial.printf("[Firebase] Failed to set status: %s\n", fbdo.errorReason().c_str());
    }
    // Initialize relays to 0 (OFF)
    for (int i = 0; i < 4; i++) {
      String p = basePath + "/relays/" + String(relayNames[i]);
      if (!Firebase.RTDB.setInt(&fbdo, p, 0)) {
        Serial.printf("[Firebase] Failed to init %s: %s\n", relayNames[i], fbdo.errorReason().c_str());
      }
    }
  }

  Serial.println(F("[System] Ready. Polling every 2 sec."));
}

void pollRelays() {
  if (!Firebase.ready()) {
    // nothing to do until Firebase is ready
    return;
  }

  String basePath = "/devices/" + deviceID + "/relays";

  for (int i = 0; i < 4; i++) {
    String fullPath = basePath + "/" + relayNames[i];

    // READ from Firebase
    if (!Firebase.RTDB.getInt(&fbdo, fullPath)) {
      Serial.printf("[Firebase] Read failed %s: %s\n", relayNames[i], fbdo.errorReason().c_str());
      continue;
    }

    int desired = fbdo.intData();   // 1 = ON, 0 = OFF
    bool shouldTurnOn = (desired == 1);

    // ACTIVE LOW mapping: ON -> LOW, OFF -> HIGH
    int desiredPinState = shouldTurnOn ? LOW : HIGH;

    // Only update if current pin state differs
    int currentPinState = digitalRead(relayPins[i]);
    if (currentPinState != desiredPinState) {
      digitalWrite(relayPins[i], desiredPinState);
      Serial.printf("[Relay] %s → %s\n", relayNames[i], shouldTurnOn ? "ON" : "OFF");

      // Confirm by writing the integer back to DB (keeps type consistent)
      if (!Firebase.RTDB.setInt(&fbdo, fullPath, desired)) {
        Serial.printf("[Firebase] Failed to confirm state for %s: %s\n", relayNames[i], fbdo.errorReason().c_str());
      }
    }
  }
}

void loop() {
  // ensure connectivity (non-blocking-ish)
  ensureWiFiSafe();
  ensureFirebaseSafe();

  if (millis() - lastPollTime >= POLL_INTERVAL) {
    pollRelays();
    lastPollTime = millis();
  }

  // keep short delay to allow background tasks
  delay(10);
}
