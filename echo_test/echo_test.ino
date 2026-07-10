void setup() {
  *(unsigned int *)0x40023840 |= (1 << 19); // RCC_APB1ENR: UART4EN
  *(unsigned int *)0x40004C08 = 139;         // UART4->BRR = 115200@16MHz
  *(unsigned int *)0x40004C0C = 0x200C;      // UART4->CR1 = UE|TE|RE

  const char *m = "Echo ready\n";
  volatile unsigned int *sr = (unsigned int *)0x40004C00;
  volatile unsigned int *dr = (unsigned int *)0x40004C04;
  for (const char *p = m; *p; p++) { while (!(*sr & 0x80)); *dr = *p; }
}

void loop() {
  volatile unsigned int *sr = (unsigned int *)0x40004C00;
  volatile unsigned int *dr = (unsigned int *)0x40004C04;

  if (*sr & (1 << 5)) {
    unsigned char c = *dr;
    while (!(*sr & 0x80));
    *dr = c;
  }
}
