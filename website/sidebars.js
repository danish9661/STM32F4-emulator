/** @type {import('@docusaurus/plugin-content-docs').SidebarsConfig} */
const sidebars = {
  docsSidebar: [
    'intro',
    {
      type: 'category',
      label: 'Getting Started',
      collapsed: false,
      items: ['usage', 'networking', 'components'],
    },
    {
      type: 'category',
      label: 'Boards',
      collapsed: false,
      items: ['boards', 'boards/stm32f401', 'boards/stm32f411', 'boards/stm32f407', 'boards/stm32f407ve', 'boards/stm32f429'],
    },
    {
      type: 'category',
      label: 'Architecture',
      collapsed: false,
      items: ['architecture', 'peripherals'],
    },
    {
      type: 'category',
      label: 'Advanced',
      collapsed: false,
      items: ['benchmarks', 'mcp', 'progress-and-future'],
    },
  ],
};

module.exports = sidebars;
