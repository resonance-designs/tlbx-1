// @ts-check
const lightCodeTheme = require('prism-react-renderer/themes/github');
const darkCodeTheme = require('prism-react-renderer/themes/dracula');

/** @type {import('@docusaurus/types').Config} */
const config = {
  title: 'GrainRust Docs',
  tagline: 'Documentation for the GrainRust sampler',
  url: 'https://grainrust.local',
  baseUrl: '/',
  onBrokenLinks: 'throw',
  onBrokenMarkdownLinks: 'warn',
  organizationName: 'grainrust',
  projectName: 'grainrust',
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
      title: 'GrainRust',
      items: [
        { to: '/docs/intro', label: 'Developer Docs', position: 'left' },
        { to: '/', label: 'Docs Home', position: 'left' }
      ]
    },
    prism: {
      theme: lightCodeTheme,
      darkTheme: darkCodeTheme
    }
  }
};

module.exports = config;
