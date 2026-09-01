import React from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import styles from './index.module.css';

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <header className={styles.heroBanner}>
      <div className={styles.heroGlow} />
      <div className={styles.heroGrid} />
      <div className="container">
        <div className={styles.heroBadge}>Open Source · Browser + Node · WebAssembly</div>
        <h1 className={styles.heroTitle}>
          <span className={styles.heroTitleGrad}>STM32F4 Emulator</span>
        </h1>
        <p className={styles.heroSubtitle}>
          Real Cortex-M4 firmware on an emulated MCU — Unicorn CPU + Rust peripherals, all WebAssembly
        </p>
        <div className={styles.buttons}>
          <Link
            className={clsx('button button--lg', styles.btnPrimary)}
            to="/docs/">
            Documentation
          </Link>
          <Link
            className={clsx('button button--lg', styles.btnSecondary)}
            href="/stm32F4-emulator/console/">
            Live Demo →
          </Link>
        </div>
        <div className={styles.heroStats}>
          <div className={styles.heroStat}>
            <span className={styles.heroStatValue}>33</span>
            <span className={styles.heroStatLabel}>Peripherals</span>
          </div>
          <div className={styles.heroStatDivider} />
          <div className={styles.heroStat}>
            <span className={styles.heroStatValue}>44</span>
            <span className={styles.heroStatLabel}>Firmware demos</span>
          </div>
          <div className={styles.heroStatDivider} />
          <div className={styles.heroStat}>
            <span className={styles.heroStatValue}>~25</span>
            <span className={styles.heroStatLabel}>FPS DOOM</span>
          </div>
          <div className={styles.heroStatDivider} />
          <div className={styles.heroStat}>
            <span className={styles.heroStatValue}>0</span>
            <span className={styles.heroStatLabel}>Native deps</span>
          </div>
        </div>
      </div>
    </header>
  );
}

function Features() {
  const features = [
    {
      title: 'Real Firmware',
      description: 'Runs actual Cortex-M4 Thumb-2 binaries — the same firmware works on real STM32F407 hardware.',
      icon: '🔧',
      color: '#34d399',
    },
    {
      title: '33 Peripherals',
      description: 'USART, GPIO, DMA, ETH, TIM, CAN, ADC, DAC, I2S, LTDC, SPI, I2C — all modeled in Rust.',
      icon: '⚡',
      color: '#fbbf24',
    },
    {
      title: 'Browser + Node',
      description: 'Runs headless in Node.js or in a browser tab via WebAssembly. No native dependencies.',
      icon: '🌐',
      color: '#60a5fa',
    },
    {
      title: 'Real Networking',
      description: 'DHCP + TCP + HTTP against a real gVisor network stack or a deterministic script.',
      icon: '🔗',
      color: '#22d3ee',
    },
    {
      title: 'DOOM',
      description: 'The full DOOM 1 shareware running at ~25 FPS in your browser, with audio.',
      icon: '💀',
      color: '#f87171',
    },
    {
      title: 'AI Integration',
      description: 'MCP server lets AI agents boot firmware, step execution, and inspect registers interactively.',
      icon: '🤖',
      color: '#a78bfa',
    },
  ];

  return (
    <section className={styles.features}>
      <div className="container">
        <h2 className={styles.sectionTitle}>Everything you need</h2>
        <p className={styles.sectionSubtitle}>A complete embedded development environment in your browser</p>
        <div className={styles.featureGrid}>
          {features.map((f, idx) => (
            <div key={idx} className={styles.featureCard} style={{'--accent': f.color}}>
              <div className={styles.featureIcon} style={{background: f.color + '15', color: f.color}}>{f.icon}</div>
              <h3 className={styles.featureTitle}>{f.title}</h3>
              <p className={styles.featureDesc}>{f.description}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function Architecture() {
  return (
    <section className={styles.archSection}>
      <div className="container">
        <h2 className={styles.sectionTitle}>How it works</h2>
        <p className={styles.sectionSubtitle}>Three layers, zero magic</p>
        <div className={styles.archGrid}>
          <div className={styles.archCard}>
            <div className={styles.archNum}>01</div>
            <h3>Unicorn CPU</h3>
            <p>QEMU-derived ARM Cortex-M4 core compiled to WASM. Executes real Thumb-2 firmware binaries instruction by instruction.</p>
            <div className={styles.archTag}>unicorn_arm.cjs · 837 KB</div>
          </div>
          <div className={styles.archArrow}>→</div>
          <div className={styles.archCard}>
            <div className={styles.archNum}>02</div>
            <h3>Rust Peripherals</h3>
            <p>33 peripheral modules modeled in Rust, compiled to WASM with wasm-bindgen. Register-mapped MMIO with real interrupt logic.</p>
            <div className={styles.archTag}>stm32_periph_wasm · 1.1 MB</div>
          </div>
          <div className={styles.archArrow}>→</div>
          <div className={styles.archCard}>
            <div className={styles.archNum}>03</div>
            <h3>JS Driver</h3>
            <p>Memory hooks route MMIO reads/writes between CPU and peripherals. Drives UART output, ETH frames, GPIO state, and more.</p>
            <div className={styles.archTag}>emulator.js · 1800 lines</div>
          </div>
        </div>
      </div>
    </section>
  );
}

function QuickStart() {
  return (
    <section className={styles.quickStart}>
      <div className="container">
        <h2 className={styles.sectionTitle}>Get started in 30 seconds</h2>
        <div className={styles.codeGrid}>
          <div className={styles.codeCard}>
            <div className={styles.codeLabel}>Browser</div>
            <pre className={styles.codeBlock}>
              <code>{`# Serve the site
npx serve site/

# Open http://localhost:3000
# Pick a firmware, click Boot`}</code>
            </pre>
          </div>
          <div className={styles.codeCard}>
            <div className={styles.codeLabel}>Node.js</div>
            <pre className={styles.codeBlock}>
              <code>{`npm install stm32f4-emu

# Run CLI
npx stm32f4-emu firmware.bin

# Or programmatic
import { STM32F4 } from 'stm32f4-emu'
const mcu = await STM32F4.create({ firmware })`}</code>
            </pre>
          </div>
        </div>
      </div>
    </section>
  );
}

export default function Home() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <Layout
      title="Documentation"
      description={siteConfig.tagline}>
      <HomepageHeader />
      <main>
        <Features />
        <Architecture />
        <QuickStart />
      </main>
    </Layout>
  );
}
