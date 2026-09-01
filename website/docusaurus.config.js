// @ts-check

/** @type {import('@docusaurus/types').Config} */
const config = {
  title: 'STM32F4 Emulator',
  tagline: 'Real Cortex-M4 firmware on an emulated MCU — Unicorn CPU + Rust peripherals, all WebAssembly',
  favicon: 'img/favicon.ico',

  url: 'https://danish9661.github.io',
  baseUrl: '/stm32F4-emulator/',

  organizationName: 'danish9661',
  projectName: 'stm32F4-emulator',
  deploymentBranch: 'gh-pages',
  trailingSlash: false,

  onBrokenLinks: 'warn',
  onBrokenMarkdownLinks: 'warn',

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      /** @type {import('@docusaurus/preset-classic').Options} */
      ({
        docs: {
          sidebarPath: './sidebars.js',
          editUrl: 'https://github.com/danish9661/stm32F4-emulator/tree/master/website/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      }),
    ],
  ],

  themeConfig:
    /** @type {import('@docusaurus/preset-classic').ThemeConfig} */
    ({
      image: 'img/social-card.png',
      navbar: {
        title: 'STM32F4 Emulator',
        logo: {
          alt: 'STM32F4 Logo',
          src: 'img/logo.svg',
        },
        items: [
          {
            type: 'docSidebar',
            sidebarId: 'docsSidebar',
            position: 'left',
            label: 'Documentation',
          },
          {
            href: 'https://danish9661.github.io/stm32F4-emulator/',
            label: 'Live Demo',
            position: 'left',
          },
          {
            href: 'https://github.com/danish9661/stm32F4-emulator',
            label: 'GitHub',
            position: 'right',
          },
          {
            href: 'https://www.npmjs.com/package/stm32f4-emu',
            label: 'npm',
            position: 'right',
          },
        ],
      },
      footer: {
        style: 'dark',
        links: [
          {
            title: 'Docs',
            items: [
              { label: 'Architecture', to: '/docs/architecture' },
              { label: 'Usage', to: '/docs/usage' },
              { label: 'Peripherals', to: '/docs/peripherals' },
              { label: 'Components', to: '/docs/components' },
            ],
          },
          {
            title: 'Community',
            items: [
              { label: 'GitHub', href: 'https://github.com/danish9661/stm32F4-emulator' },
              { label: 'npm', href: 'https://www.npmjs.com/package/stm32f4-emu' },
            ],
          },
          {
            title: 'More',
            items: [
              { label: 'Live Demo', href: 'https://danish9661.github.io/stm32F4-emulator/' },
              { label: 'DOOM', href: 'https://danish9661.github.io/stm32F4-emulator/doom.html' },
            ],
          },
        ],
        copyright: `Copyright ${new Date().getFullYear()} Danish. Built with Docusaurus.`,
      },
      prism: {
        theme: require('prism-react-renderer').themes.github,
        darkTheme: require('prism-react-renderer').themes.dracula,
        additionalLanguages: ['bash', 'rust', 'toml', 'yaml', 'json', 'c'],
      },
      colorMode: {
        defaultMode: 'dark',
        disableSwitch: false,
        respectPrefersColorScheme: true,
      },
    }),
};

module.exports = config;
