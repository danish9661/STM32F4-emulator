import React, {useEffect} from 'react';
import Head from '@docusaurus/Head';
import Link from '@docusaurus/Link';
import Layout from '@theme/Layout';

// Single-landing policy: the live launcher lives in site/ (served as
// /console/ on Pages, as / locally). This root page only redirects there
// (plus a manual link for no-JS/crawlers). Docs under /docs/ are unaffected.
const TARGET = '/stm32F4-emulator/console/';

export default function Home() {
  useEffect(() => {
    window.location.replace(TARGET);
  }, []);
  return (
    <Layout
      title="Redirecting…"
      description="The STM32F4 live demo moved to /console/">
      <Head>
        <meta httpEquiv="refresh" content={`0; url=${TARGET}`} />
        <link rel="canonical" href={TARGET} />
      </Head>
      <main className="container" style={{padding: '4rem 0', textAlign: 'center'}}>
        <p>
          Redirecting to the <Link href={TARGET}>live demo</Link>…
        </p>
      </main>
    </Layout>
  );
}
