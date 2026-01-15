module.exports = {
  stories: ['../stories/**/*.mdx'],
  addons: ['@storybook/addon-docs'],
  framework: {
    name: '@storybook/html',
    options: {}
  },
  docs: {
    autodocs: false
  }
};
