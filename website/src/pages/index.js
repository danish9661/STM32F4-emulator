import React from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import styles from './index.module.css';

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <header className={clsx('hero hero--primary', styles.heroBanner)}>
      <div className="container">
        <h1 className="hero__title">{siteConfig.title}</h1>
        <p className="hero__subtitle">{siteConfig.tagline}</p>
        <div className={styles.buttons}>
          <Link
            className="button button--secondary button--lg"
            to="/docs/">
            Read the Docs
          </Link>
          <Link
            className="button button--outline button--lg"
            href="/stm32F4-emulator/console/"
            style={{marginLeft: '1rem', borderColor: 'white', color: 'white'}}>
            Live Demo
          </Link>
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
    },
    {
      title: '33 Peripherals',
      description: 'USART, GPIO, DMA, ETH, TIM, CAN, ADC, DAC, I2S, LTDC, SPI, I2C — all modeled in Rust.',
      icon: '⚡',
    },
    {
      title: 'Browser + Node',
      description: 'Runs headless in Node.js or in a browser tab via WebAssembly. No native dependencies.',
      icon: '🌐',
    },
    {
      title: 'Real Networking',
      description: 'DHCP + TCP + HTTP against a real gVisor network stack or a deterministic script.',
      icon: '🔗',
    },
    {
      title: 'DOOM',
      description: 'The full DOOM 1 shareware running at ~25 FPS in your browser, with audio.',
      icon: '💀',
    },
    {
      title: 'AI Integration',
      description: 'MCP server lets AI agents boot firmware, step execution, and inspect registers interactively.',
      icon: '🤖',
    },
  ];

  return (
    <section className={styles.features}>
      <div className="container">
        <div className="row">
          {features.map((f, idx) => (
            <div key={idx} className={clsx('col col--4')}>
              <div className={styles.featureCard}>
                <div className={styles.featureIcon}>{f.icon}</div>
                <h3>{f.title}</h3>
                <p>{f.description}</p>
              </div>
            </div>
          ))}
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
      </main>
    </Layout>
  );
}
