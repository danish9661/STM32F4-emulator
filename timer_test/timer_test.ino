volatile unsigned int seconds = 0;

void setup() {
  *(unsigned int *)0x40023830 |= (1 << 0);  // RCC_AHB1ENR: GPIOAEN
  *(unsigned int *)0x40020000 |= (1 << 10); // GPIOA_MODER: PA5 output
  *(unsigned int *)0x40023840 |= (1 << 19); // RCC_APB1ENR: UART4EN
  *(unsigned int *)0x40004C08 = 139;         // UART4->BRR = 115200@16MHz
  *(unsigned int *)0x40004C0C = 0x200C;      // UART4->CR1 = UE|TE|RE

  *(unsigned int *)0x40023840 |= (1 << 0);  // RCC_APB1ENR: TIM2EN
  *(unsigned int *)0x40000028 = 15999;       // TIM2->PSC = 16000-1
  *(unsigned int *)0x4000002C = 999;         // TIM2->ARR = 1000-1
  *(unsigned int *)0x40000000 = 1;           // TIM2->CR1 = CEN

  const char *m = "Timer start\n";
  volatile unsigned int *sr = (unsigned int *)0x40004C00;
  volatile unsigned int *dr = (unsigned int *)0x40004C04;
  for (const char *p = m; *p; p++) { while (!(*sr & 0x80)); *dr = *p; }
}

void loop() {
  volatile unsigned int *tim_sr = (unsigned int *)0x40000010;
  volatile unsigned int *bsrr = (unsigned int *)0x40020018;
  volatile unsigned int *sr = (unsigned int *)0x40004C00;
  volatile unsigned int *dr = (unsigned int *)0x40004C04;

  if (*tim_sr & 1) {
    *tim_sr = ~1;
    seconds++;
    bsrr[0] = (1 << 5) | (1 << 21);
    unsigned int s = seconds;
    const char *digits = "0123456789";
    char buf[16];
    int i = 0, j = 0;
    if (s == 0) { buf[i++] = '0'; }
    else { while (s) { buf[i++] = digits[s % 10]; s /= 10; } }
    const char *pre = "t=";
    for (const char *p = pre; *p; p++) { while (!(*sr & 0x80)); *dr = *p; }
    for (int k = i - 1; k >= 0; k--) { while (!(*sr & 0x80)); *dr = buf[k]; }
    const char *suf = "s\n";
    for (const char *p = suf; *p; p++) { while (!(*sr & 0x80)); *dr = *p; }
  }
}
