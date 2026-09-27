import React from 'react';
import Link from '@docusaurus/Link';
import Layout from '@theme/Layout';
import styles from './index.module.css';

// Root landing: the splash/home page lives HERE (rendered by Docusaurus at
// /STM32F4-emulator/), with buttons to the emulator UI that pages.yml
// copies flat from site/ — /console.html, /doom.html, /docs.html.
export default function Home() {
  return (
    <Layout
      title="Live console, DOOM, 223 firmware builds"
      description="STM32F4 emulator: run real Cortex-M4 firmware in a browser tab — Rust CPU + peripheral model in WebAssembly.">
      <main className={styles.heroBanner}>
        <h1 className={styles.heroTitle}>STM32F4 Emulator</h1>
        <p className={styles.heroSubtitle}>
          Real Cortex-M4 firmware on an emulated MCU — Rust CPU + Rust
          peripherals, all WebAssembly. No install, no plugins: it runs in
          this tab.
        </p>
        <div className={styles.buttons}>
          <Link className={styles.btnPrimary} href="/STM32F4-emulator/console.html">
            Launch Console →
          </Link>
          <Link className={styles.btnSecondary} href="/STM32F4-emulator/doom.html">
            ▶ Play DOOM
          </Link>
          <Link className={styles.btnSecondary} to="/docs/usage">
            Docs
          </Link>
        </div>
        <div className={styles.heroStats}>
          <div className={styles.heroStat}>
            <span className={styles.heroStatValue}>41</span>
            <span className={styles.heroStatLabel}>Peripherals</span>
          </div>
          <div className={styles.heroStatDivider} />
          <div className={styles.heroStat}>
            <span className={styles.heroStatValue}>223</span>
            <span className={styles.heroStatLabel}>Firmware builds</span>
          </div>
          <div className={styles.heroStatDivider} />
          <div className={styles.heroStat}>
            <span className={styles.heroStatValue}>5</span>
            <span className={styles.heroStatLabel}>Chips</span>
          </div>
          <div className={styles.heroStatDivider} />
          <div className={styles.heroStat}>
            <span className={styles.heroStatValue}>35</span>
            <span className={styles.heroStatLabel}>FPS DOOM</span>
          </div>
        </div>
        <p className={styles.heroSubtitle} style={{ marginTop: '2rem', fontSize: '0.9rem' }}>
          Try now — no boot needed:{' '}
          <Link href="/STM32F4-emulator/console.html?fw=blinky">blinky</Link> ·{' '}
          <Link href="/STM32F4-emulator/console.html?fw=eth_http">eth_http</Link> ·{' '}
          <Link href="/STM32F4-emulator/console.html?fw=can_test">can_test</Link> ·{' '}
          <Link href="/STM32F4-emulator/console.html?fw=usb_cdc_test">usb_cdc_test</Link>
        </p>
      </main>
    </Layout>
  );
}
