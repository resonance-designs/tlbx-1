// @ts-check

/** @type {import('@docusaurus/types').Config} */
const config = {
  title: 'TLBX-1 Docs',
  tagline: 'Documentation for the TLBX-1 Audio Toolbox',
  url: 'https://tlbx-1.local',
  baseUrl: './',
  onBrokenLinks: 'warn',
  markdown: {
    hooks: {
      onBrokenMarkdownLinks: 'warn'
    }
  },
  organizationName: 'resonancedesigns',
  projectName: 'tlbx-1',
  presets: [
    [
      'classic',
      {
        docs: {
          path: '../docs',
          routeBasePath: 'docs',
          sidebarPath: require.resolve('./sidebars.js')
        },
        blog: false,
        theme: {
          customCss: require.resolve('./src/css/custom.css')
        }
      }
    ]
  ],
  themeConfig: {
    navbar: {
      title: 'TLBX-1 Docs',
      items: [
        { to: '/docs/intro', label: 'Developer Docs', position: 'left' },
        { to: '/', label: 'Docs Home', position: 'left' }
      ]
    },
    prism: {}
  }
};

module.exports = config;
