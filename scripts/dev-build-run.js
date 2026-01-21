const { spawnSync } = require('child_process');
const fs = require('fs');
const path = require('path');

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: 'inherit', ...options });
  if (result.error) {
    console.error(result.error);
  }
  if (result.status !== 0) {
    process.exit(result.status || 1);
  }
}

function readTargetDir(root) {
  const envDir = process.env.CARGO_TARGET_DIR;
  if (envDir) {
    return envDir;
  }
  const configPath = path.join(root, '.cargo', 'config.toml');
  if (fs.existsSync(configPath)) {
    const config = fs.readFileSync(configPath, 'utf8');
    const match = config.match(/^target-dir\s*=\s*"([^"]+)"/m);
    if (match) {
      return path.resolve(root, match[1]);
    }
  }
  return path.join(root, 'target');
}

const root = path.resolve(__dirname, '..');
const targetDir = readTargetDir(root);
const profile = 'debug';
const exePath = path.join(targetDir, profile, 'tlbx-1.exe');
const depsDir = path.join(targetDir, profile, 'deps');
const docsSite = path.join(root, 'docs-site');
const docsBuild = path.join(docsSite, 'build');
const docsDest = path.join(targetDir, profile, 'documentation');

console.log(`Using target dir: ${targetDir}`);
run('cargo', ['build', '--bin', 'tlbx-1'], { cwd: root });
console.log('Cargo build complete.');

console.log('Installing docs dependencies...');
run('npm', ['--prefix', 'docs-site', 'install'], { cwd: root, shell: true });
console.log('Building docs site...');
run('npm', ['--prefix', 'docs-site', 'run', 'build'], { cwd: root, shell: true });

if (fs.existsSync(docsDest)) {
  fs.rmSync(docsDest, { recursive: true, force: true });
}
fs.mkdirSync(docsDest, { recursive: true });
fs.cpSync(docsBuild, docsDest, { recursive: true });

console.log(`Docs deployed to ${docsDest}`);

if (!fs.existsSync(exePath)) {
  const candidates = fs.existsSync(depsDir)
    ? fs.readdirSync(depsDir).filter(name => name.startsWith('tlbx-1') && name.endsWith('.exe'))
    : [];
  if (candidates.length === 0) {
    console.error(`Executable not found: ${exePath}`);
    console.error(`Checked deps dir: ${depsDir}`);
    process.exit(1);
  }
  const fallback = path.join(depsDir, candidates[0]);
  console.warn(`Using fallback exe: ${fallback}`);
  run(fallback, [], { cwd: root });
} else {
  run(exePath, [], { cwd: root });
}
