void setup() {
  *(unsigned int *)0x40023830 |= (1 << 0);  // RCC_AHB1ENR: GPIOAEN
  *(unsigned int *)0x40020000 |= (1 << 10); // GPIOA_MODER: PA5 output
  *(unsigned int *)0x40023840 |= (1 << 19); // RCC_APB1ENR: UART4EN
  *(unsigned int *)0x40004C08 = 139;         // UART4->BRR = 115200@16MHz
  *(unsigned int *)0x40004C0C = 0x200C;      // UART4->CR1 = UE|TE|RE
  volatile unsigned int *sr = (unsigned int *)0x40004C00;
  volatile unsigned int *dr = (unsigned int *)0x40004C04;
  const char *msg = "Hello from UART4!\n";
  for (const char *p = msg; *p; p++) {
    while (!(*sr & 0x80));
    *dr = *p;
  }
}

void loop() {
  volatile unsigned int *bsrr = (unsigned int *)0x40020018;
  volatile unsigned int *sr = (unsigned int *)0x40004C00;
  volatile unsigned int *dr = (unsigned int *)0x40004C04;
  bsrr[0] = (1 << 5);     // PA5 HIGH
  for (volatile int i = 0; i < 200000; i++);
  bsrr[0] = (1 << 21);    // PA5 LOW
  for (volatile int i = 0; i < 200000; i++);
  const char *msg = "loop\n";
  for (const char *p = msg; *p; p++) {
    while (!(*sr & 0x80));
    *dr = *p;
  }
}
