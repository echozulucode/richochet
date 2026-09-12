#!/usr/bin/env node
/**
 * bump-version.mjs — one version, three files.
 *
 * Richochet's version is mirrored across three places that must agree:
 *
 *   1. package.json                  "version"
 *   2. Cargo.toml                    [workspace.package] version  (mdcore, mdcli and app inherit it)
 *   3. src-tauri/tauri.conf.json     "version"
 *
 * #3 is the one that matters most at runtime: it is what `getVersion()` reports and what the
 * updater compares against the release manifest. If it drifts below the others the app either
 * nags forever or never sees an update at all, so `just bump` is the only sanctioned way to
 * change any of them, and CI re-checks all three against the tag before building a release.
 *
 * Usage:
 *
 *   just bump 0.2.0          # explicit version
 *   just bump patch          # 0.1.0 -> 0.1.1
 *   just bump minor          # 0.1.0 -> 0.2.0
 *   just bump major          # 0.1.0 -> 1.0.0
 *   just bump patch --tag    # also commit the three files and create an annotated tag
 *
 * `--tag` stops short of pushing, deliberately: the tag push is what triggers the release
 * workflow, and a typo in a version should be fixable with `git tag -d` rather than by
 * publishing a release nobody wanted.
 *
 *   git push && git push origin v<version>
 *
 * Run with `--check <version>` to verify the three files agree with a version instead of
 * writing anything. That is the mode CI uses; it exits non-zero on the first mismatch.
 */

import { readFile, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/**
 * Each file needs its own surgical regex rather than a parse-and-reserialize: rewriting
 * tauri.conf.json through JSON.parse would reformat the whole file, and a TOML round trip would
 * drop the comments in Cargo.toml. Replacing one quoted value in place keeps the diff to the
 * single line that actually changed.
 */
const FILES = [
  {
    path: 'package.json',
    // The first top-level "version" key. Dependency versions live inside nested objects further
    // down, and the non-greedy first match never reaches them.
    pattern: /("version"\s*:\s*)"([^"]+)"/,
  },
  {
    path: 'Cargo.toml',
    // Anchored to [workspace.package] so the `version = "..."` strings in
    // [workspace.dependencies] below it are untouchable.
    pattern: /(\[workspace\.package\][\s\S]*?\nversion\s*=\s*)"([^"]+)"/,
  },
  {
    path: 'src-tauri/tauri.conf.json',
    pattern: /("version"\s*:\s*)"([^"]+)"/,
  },
];

const SEMVER_RE = /^(\d+)\.(\d+)\.(\d+)(?:-([\w.-]+))?$/;

function parseSemver(value) {
  const match = SEMVER_RE.exec(value);
  if (!match) return null;
  return { major: +match[1], minor: +match[2], patch: +match[3], pre: match[4] ?? null };
}

function bumpSemver(current, kind) {
  const version = parseSemver(current);
  if (!version) throw new Error(`current version "${current}" is not semver`);
  switch (kind) {
    case 'patch':
      return `${version.major}.${version.minor}.${version.patch + 1}`;
    case 'minor':
      return `${version.major}.${version.minor + 1}.0`;
    case 'major':
      return `${version.major + 1}.0.0`;
    default:
      throw new Error(`unknown bump kind: ${kind}`);
  }
}

/** Read the version each file currently declares, so a drift is reported rather than papered over. */
async function readVersions() {
  const versions = [];
  for (const { path, pattern } of FILES) {
    const raw = await readFile(join(repoRoot, path), 'utf8');
    const match = pattern.exec(raw);
    if (!match) {
      throw new Error(`no version found in ${path} — its structure may have changed`);
    }
    versions.push({ path, version: match[2] });
  }
  return versions;
}

async function writeVersion(file, newVersion) {
  const absolute = join(repoRoot, file.path);
  const raw = await readFile(absolute, 'utf8');
  const updated = raw.replace(file.pattern, `$1"${newVersion}"`);
  if (updated === raw) throw new Error(`no version found in ${file.path}`);
  await writeFile(absolute, updated);
}

function git(args) {
  const result = spawnSync('git', args, { cwd: repoRoot, stdio: 'inherit', encoding: 'utf8' });
  if (result.status !== 0) throw new Error(`git ${args.join(' ')} failed`);
}

/** `--check <version>`: assert all three files already say <version>. Used by the release workflow. */
async function check(expected) {
  const wanted = expected.replace(/^v/, '');
  if (!parseSemver(wanted)) {
    console.error(`--check needs a semver version, got "${expected}"`);
    return 1;
  }
  const versions = await readVersions();
  const wrong = versions.filter((entry) => entry.version !== wanted);
  for (const { path, version } of versions) {
    console.log(`  ${version === wanted ? '✓' : '✗'} ${path} — ${version}`);
  }
  if (wrong.length > 0) {
    console.error(
      `\nExpected every file to declare ${wanted}. Run \`just bump ${wanted}\` and commit the result.`,
    );
    return 1;
  }
  console.log(`\nAll three files declare ${wanted}.`);
  return 0;
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length === 0) {
    console.error('Usage: just bump <version | patch | minor | major> [--tag]');
    console.error('       just bump --check <version>');
    return 1;
  }

  if (args[0] === '--check') {
    if (!args[1]) {
      console.error('Usage: just bump --check <version>');
      return 1;
    }
    return await check(args[1]);
  }

  const request = args[0];
  const shouldTag = args.includes('--tag');

  // Every file has to start in agreement, or "the current version" is not a single answer and a
  // bump would quietly resolve the drift to whatever package.json happened to say.
  const versions = await readVersions();
  const distinct = [...new Set(versions.map((entry) => entry.version))];
  if (distinct.length > 1) {
    console.error('The three version files disagree before bumping:');
    for (const { path, version } of versions) console.error(`  ${path} — ${version}`);
    console.error('\nSet them to one version explicitly: just bump <version>');
    if (!parseSemver(request)) return 1;
  }
  const current = versions[0].version;

  let newVersion;
  if (['patch', 'minor', 'major'].includes(request)) {
    newVersion = bumpSemver(current, request);
  } else if (parseSemver(request.replace(/^v/, ''))) {
    newVersion = request.replace(/^v/, '');
  } else {
    console.error(
      `Invalid version "${request}". Expected semver (e.g. 0.2.0), or patch/minor/major.`,
    );
    return 1;
  }

  if (newVersion === current && distinct.length === 1) {
    console.error(`Already at ${newVersion}. Nothing to do.`);
    return 1;
  }

  console.log(`Bumping ${current} → ${newVersion}`);
  for (const file of FILES) {
    await writeVersion(file, newVersion);
    console.log(`  ✓ ${file.path}`);
  }

  if (shouldTag) {
    console.log('\nStaging, committing, tagging…');
    git(['add', ...FILES.map((file) => file.path)]);
    git(['commit', '-m', `chore: bump to v${newVersion}`]);
    git(['tag', '-a', `v${newVersion}`, '-m', `v${newVersion}`]);
    console.log(`\nTagged v${newVersion}. Nothing is pushed yet — review, then:`);
    console.log(`  git push && git push origin v${newVersion}`);
  } else {
    console.log('\nDone. Review with: git diff');
    console.log('Then, to release:');
    console.log(`  git commit -am "chore: bump to v${newVersion}"`);
    console.log(`  git tag -a v${newVersion} -m "v${newVersion}"`);
    console.log(`  git push && git push origin v${newVersion}`);
    console.log('\nOr re-run with --tag to do the commit and tag in one step.');
  }
  return 0;
}

main().then(
  (code) => process.exit(code),
  (error) => {
    console.error(error.stack ?? error.message ?? error);
    process.exit(1);
  },
);
