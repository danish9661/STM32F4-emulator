// Real-firmware board validation: standard Arduino sketch exercising
// the full Arduino core stack — SystemInit clock tree (RCC PLL + ready
// flags), .data/.bss init, GPIO digitalWrite, SysTick-driven delay(),
// and Serial prints. Same sketch for all 8 boards (LED_BUILTIN + Serial
// resolve per variant).
void setup() {
  Serial.begin(115200);
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.println("=== Arduino Board Test ===");
}

void loop() {
  static int n = 0;
  digitalWrite(LED_BUILTIN, HIGH);
  Serial.print("tick ");
  Serial.print(n);
  Serial.println(" LED=ON");
  delay(100);
  digitalWrite(LED_BUILTIN, LOW);
  Serial.print("tick ");
  Serial.print(n);
  Serial.println(" LED=OFF");
  delay(100);
  if (++n >= 3) {
    Serial.println("Arduino done");
    while (1);
  }
}
