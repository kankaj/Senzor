import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/** The Senzor version, read from the workspace Cargo.toml so it has one source. */
export const version: string = (() => {
  const cargoTomlPath = join(process.cwd(), '..', 'Cargo.toml');
  let cargoToml: string;
  try {
    cargoToml = readFileSync(cargoTomlPath, 'utf8');
  } catch (cause) {
    throw new Error(
      `cannot read ${cargoTomlPath}: run the site from website/ inside the Senzor repository (cwd is ${process.cwd()})`,
      { cause },
    );
  }
  const afterHeader = cargoToml.split(/^\[workspace\.package\]\s*$/m)[1] ?? '';
  const section = afterHeader.split(/^\[/m)[0];
  const match = section.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) {
    throw new Error('no version under [workspace.package] in ../Cargo.toml');
  }
  return match[1];
})();
