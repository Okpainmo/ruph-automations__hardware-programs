#include <WiFi.h>
#include <Firebase_ESP_Client.h>
#include "addons/TokenHelper.h"
#include "addons/RTDBHelper.h"

// ====== Config ======
#define WIFI_SSID       "Infinix NOTE 30 VIP"
#define WIFI_PASSWORD   "12345678p"
#define API_KEY         "AIzaSyAkJcFWMqWUMbkKidbnd_lYCjhLEv5AiGI"
#define DATABASE_URL    "https://andrew-5d2ad-default-rtdb.firebaseio.com/"

#define RELAY4 16
#define RELAY3 17
#define RELAY2 5
#define RELAY1 18
#define STATUS_LED 2

// Relay pins array
const int relayPins[4] = { RELAY1, RELAY2, RELAY3, RELAY4 };

FirebaseData fbdo;
FirebaseAuth auth;
FirebaseConfig config;
String deviceID;

// Polling interval
unsigned long lastPollTime = 0;
const unsigned long POLL_INTERVAL = 1000;  // 1 seconds

void setup() {
  Serial.begin(115200);
  delay(100);

  // --- Wi-Fi ---
  WiFi.begin(WIFI_SSID, WIFI_PASSWORD);
  Serial.print(F("Connecting to Wi-Fi"));
  while (WiFi.status() != WL_CONNECTED) {
    Serial.print(F("."));
    delay(300);
  }
  Serial.println(F("\nConnected!"));
  Serial.printf("IP: %s\n", WiFi.localIP().toString().c_str());

  // --- Device ID ---
  uint64_t chipid = ESP.getEfuseMac();
  deviceID = "ESP32_" + String((uint32_t)(chipid >> 32), HEX) +
             String((uint32_t)(chipid & 0xFFFFFFFF), HEX);
  deviceID.toUpperCase();
  Serial.println("Device ID: " + deviceID);

  // --- Firebase ---
  config.api_key = API_KEY;
  config.database_url = DATABASE_URL;

  // Anonymous sign-up
  if (Firebase.signUp(&config, &auth, "", "")) {
    Serial.println(F("Firebase sign-up OK"));
  } else {
    Serial.printf("Sign-up failed: %s\n", config.signer.signupError.message.c_str());
  }

  Firebase.begin(&config, &auth);
  Firebase.reconnectWiFi(true);

  fbdo.setResponseSize(4096);

  // --- Relays ---
  for (int i = 0; i < 4; i++) {
    pinMode(relayPins[i], OUTPUT);
    digitalWrite(relayPins[i], LOW);  // Start ON (active LOW)
  }

  // --- Initialize Firebase DB ---
  String basePath = "/devices/" + deviceID;
  Firebase.RTDB.setString(&fbdo, basePath + "/status", "online");
  Firebase.RTDB.setInt(&fbdo, basePath + "/relays/relay1", 0);
  Firebase.RTDB.setInt(&fbdo, basePath + "/relays/relay2", 0);
  Firebase.RTDB.setInt(&fbdo, basePath + "/relays/relay3", 0);
  Firebase.RTDB.setInt(&fbdo, basePath + "/relays/relay4", 0);

  Serial.println(F("Polling mode started (every 2 sec)"));
}

void pollRelays() {
  if (!Firebase.ready()) return;

  String basePath = "/devices/" + deviceID + "/relays";
  const char* relays[] = { "relay1", "relay2", "relay3", "relay4" };

  for (int i = 0; i < 4; i++) {
    String path = basePath + "/" + relays[i];

    // READ from Firebase
    if (Firebase.RTDB.getInt(&fbdo, path.c_str())) {
      int desired = fbdo.intData();
      int current = digitalRead(relayPins[i]);

      if ((desired == 1 && current == LOW) || (desired == 0 && current == HIGH)) {
        digitalWrite(relayPins[i], desired ? HIGH : LOW);
        Serial.printf("Relay%d → %s\n", i + 1, desired ? "ON" : "OFF");
        

        // SEND BACK to Firebase (optional: confirm state)
        bool stateBool = (desired == 1);
        Firebase.RTDB.setBool(&fbdo, path.c_str(), stateBool);
      }
    } else {
      Serial.printf("Read failed %s: %s\n", relays[i], fbdo.errorReason().c_str());
    }
  }
}

void loop() {
  // Poll every 2 seconds
  if (millis() - lastPollTime >= POLL_INTERVAL) {
    pollRelays();
    lastPollTime = millis();
  }

  // Keep Firebase alive
  if (!Firebase.ready()) {
    delay(100);
    return;
  }

  delay(10);
}