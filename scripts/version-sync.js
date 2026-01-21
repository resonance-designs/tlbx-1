const fs = require('fs');
const path = require('path');

const root = path.resolve(__dirname, '..');
const cargoToml = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8');
const versionMatch = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
if (!versionMatch) {
  console.error('Failed to locate version in Cargo.toml');
  process.exit(1);
}
const version = versionMatch[1];

const headerTemplate = fs.readFileSync(path.join(root, 'scripts', 'header-template.txt'), 'utf8');

function writeHeader(filePath, component) {
  const fullPath = path.join(root, filePath);
  const contents = fs.readFileSync(fullPath, 'utf8');
  const header = headerTemplate
    .replace('{{version}}', version)
    .replace('{{component}}', component);
  const updated = contents.replace(/^\s*\/\*\*[\s\S]*?\*\/\s*/m, header + '\n');
  if (updated === contents) {
    // If no header was replaced, prepend.
    fs.writeFileSync(fullPath, header + '\n' + contents, 'utf8');
    return;
  }
  fs.writeFileSync(fullPath, updated, 'utf8');
}

function writePackageVersion(filePath) {
  const fullPath = path.join(root, filePath);
  const pkg = JSON.parse(fs.readFileSync(fullPath, 'utf8'));
  pkg.version = version;
  fs.writeFileSync(fullPath, JSON.stringify(pkg, null, 2) + '\n', 'utf8');
}

function writeReadmeVersion() {
  const readmePath = path.join(root, 'README.md');
  let text = fs.readFileSync(readmePath, 'utf8');
  const badgeLine = `![Static Badge](https://img.shields.io/badge/Version-${version}-orange)`;
  if (text.includes('![Static Badge]')) {
    text = text.replace(/^!\[Static Badge\]\(https:\/\/img\.shields\.io\/badge\/Version-[^)]+\)$/m, badgeLine);
  } else {
    text = text.replace(/^#\s+TLBX-1\s*$/m, `# TLBX-1\n\n${badgeLine}`);
  }
  fs.writeFileSync(readmePath, text, 'utf8');
}

function writeNsisVersion() {
  const nsisPath = path.join(root, 'scripts', 'packaging', 'windows', 'installer.nsi');
  let text = fs.readFileSync(nsisPath, 'utf8');
  text = text.replace(
    /^!define\s+PRODUCT_VERSION\s+"[^"]*"\s*$/m,
    `!define PRODUCT_VERSION "${version}"`
  );
  fs.writeFileSync(nsisPath, text, 'utf8');
}

writePackageVersion('package.json');
writePackageVersion(path.join('docs-site', 'package.json'));
writeReadmeVersion();
writeNsisVersion();

writeHeader(path.join('src', 'main.rs'), 'Main Entry Point');
writeHeader(path.join('src', 'lib.rs'), 'Core Logic');
writeHeader('build.rs', 'Build Script');
writeHeader(path.join('src', 'ui', 'tlbx1.slint'), 'UI Definitions');

console.log(`Synced version ${version}`);
