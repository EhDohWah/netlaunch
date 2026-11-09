// Copies the Playwright driver into src-tauri/playwright-driver/ so the
// installer can bundle it (see src-tauri/tauri.bundle.conf.json). The app
// finds it next to its exe at start-up and points playwright-rs at it.
//
// Where the driver comes from, in order:
//   1. PLAYWRIGHT_DRIVER_PATH (a folder holding node.exe and package/cli.js)
//   2. PLAYWRIGHT_DRIVER_CACHE_DIR/<driver folder> (CI)
//   3. %LOCALAPPDATA%/playwright-rust/<version>/<driver folder>, where
//      playwright-rs's build script puts it on a developer PC
//
// The version must match the pinned playwright-rs (0.20.1 bundles
// Playwright 1.64.0). Bump PLAYWRIGHT_VERSION together with the pin.
import { cpSync, existsSync, readdirSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';

const PLAYWRIGHT_VERSION = '1.64.0';
const target = resolve(import.meta.dirname, '..', 'src-tauri', 'playwright-driver');

const isDriver = (dir) => existsSync(join(dir, 'node.exe')) && existsSync(join(dir, 'package', 'cli.js'));

/** Driver folders (`playwright-1.64.0-win32_x64-node…`) directly inside `dir`. */
function driversIn(dir) {
  if (!dir || !existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((name) => name.startsWith(`playwright-${PLAYWRIGHT_VERSION}-win32_x64`))
    .map((name) => join(dir, name))
    .filter(isDriver);
}

const candidates = [
  process.env.PLAYWRIGHT_DRIVER_PATH,
  ...driversIn(process.env.PLAYWRIGHT_DRIVER_CACHE_DIR),
  ...driversIn(process.env.LOCALAPPDATA && join(process.env.LOCALAPPDATA, 'playwright-rust', PLAYWRIGHT_VERSION)),
].filter((dir) => dir && isDriver(dir));

if (!candidates.length) {
  console.error(
    `No Windows Playwright ${PLAYWRIGHT_VERSION} driver found. Run \`cargo build\` once (its build script ` +
      'downloads it), or set PLAYWRIGHT_DRIVER_PATH to a folder holding node.exe and package/cli.js.',
  );
  process.exit(1);
}

rmSync(target, { recursive: true, force: true });
cpSync(candidates[0], target, { recursive: true });
console.log(`Staged the Playwright driver from ${candidates[0]}`);
