// Arduino Wire IRQ-driver path: manual master-TX to a registered slave +
// repeated-START reads (1/2/6 byte) through the HAL IT state machine.
// Regression test for the 2026-10-06 I2C seat fix (TXE/RXNE/BTF were
// rotated, so Wire transfers never completed: endTransmission()=4,
// requestFrom()=0, manual flag-spins hung). Same sketch for all boards;
// the .bin committed under build-disco_f407vg targets I2C1 + USART2.
#include <Wire.h>
void setup() {
  Serial.begin(115200);
  Serial.println("WIRE BEGIN");
  Wire.begin();
  Serial.println("WIRE DONE");
  // 1. manual 2-byte master TX to SSD1306 0x3C (the reported hang shape)
  Wire.beginTransmission(0x3C);
  Wire.write(0x00);
  Wire.write(0xAE);
  Serial.print("TX1 ");
  Serial.println(Wire.endTransmission());
  // 2. pointer-write + repeated-START reads from regfile 0x50
  Wire.beginTransmission(0x50); Wire.write(0x00);
  Serial.print("W1 ");
  Serial.println(Wire.endTransmission(false));
  byte n1 = Wire.requestFrom(0x50, 1);
  Serial.print("N1 ");
  Serial.println(n1);
  Serial.print("B0 ");
  Serial.println(Wire.read(), HEX);
  Wire.beginTransmission(0x50); Wire.write(0x02);
  Serial.print("W2 ");
  Serial.println(Wire.endTransmission(false));
  byte n2 = Wire.requestFrom(0x50, 2);
  Serial.print("N2 ");
  Serial.println(n2);
  Serial.print("B1 ");
  Serial.print(Wire.read(), HEX);
  Serial.print(" ");
  Serial.println(Wire.read(), HEX);
  Wire.beginTransmission(0x50); Wire.write(0x00);
  Serial.print("W3 ");
  Serial.println(Wire.endTransmission(false));
  byte n6 = Wire.requestFrom(0x50, 6);
  Serial.print("N6 ");
  Serial.println(n6);
  Serial.print("B6 ");
  while (Wire.available()) { Serial.print(Wire.read(), HEX); Serial.print(" "); }
  Serial.println();
  Serial.println("WIRE ALL DONE");
}
void loop() {
  static int n = 0;
  Serial.print("WIRE LOOP ");
  Serial.println(n);
  delay(200);
  if (++n >= 2) { Serial.println("WIRE LOOP DONE"); while (1); }
}
