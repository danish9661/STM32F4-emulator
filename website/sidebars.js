/** @type {import('@docusaurus/plugin-content-docs').SidebarsConfig} */
const sidebars = {
  docsSidebar: [
    'intro',
    {
      type: 'category',
      label: 'Getting Started',
      collapsed: false,
      items: ['usage', 'components'],
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
