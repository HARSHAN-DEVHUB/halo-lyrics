// Ships a new version of Glowline as an automatic update.
//
//   npm run release            0.2.0 -> 0.2.1
//   npm run release minor      0.2.0 -> 0.3.0
//   npm run release major      0.2.0 -> 1.0.0
//   npm run release 1.4.2      exact version
//
// Bumps the version everywhere, commits "Release vX.Y.Z" and pushes to main.
// GitHub Actions (.github/workflows/release.yml) then builds the signed
// installer and publishes it; installed copies pick it up within a few hours
// (or right away with Settings → Check for updates).
import { execSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const sh = (cmd) => execSync(cmd, { encoding: "utf8" }).trim();
const run = (cmd) => execSync(cmd, { stdio: "inherit" });
const fail = (msg) => {
  console.error(`\n✖ ${msg}\n`);
  process.exit(1);
};

if (sh("git rev-parse --abbrev-ref HEAD") !== "main") fail("Switch to the main branch first.");
if (sh("git status --porcelain")) fail("Commit (or stash) your changes first, then run the release again.");

const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const current = conf.version;
const [maj, min, pat] = current.split(".").map(Number);
const arg = process.argv[2] ?? "patch";
const next =
  arg === "patch" ? `${maj}.${min}.${pat + 1}`
  : arg === "minor" ? `${maj}.${min + 1}.0`
  : arg === "major" ? `${maj + 1}.0.0`
  : /^\d+\.\d+\.\d+$/.test(arg) ? arg
  : fail(`Unknown version "${arg}". Use patch, minor, major or a version like 1.2.3.`);

const newer = (a, b) => {
  const x = a.split(".").map(Number), y = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
};
if (!newer(next, current)) fail(`${next} must be newer than the current version ${current}.`);

const edit = (file, fn) => writeFileSync(file, fn(readFileSync(file, "utf8")));
const setJsonVersion = (file, extra) =>
  edit(file, (text) => {
    const json = JSON.parse(text);
    json.version = next;
    extra?.(json);
    return JSON.stringify(json, null, 2) + "\n";
  });

setJsonVersion("src-tauri/tauri.conf.json");
setJsonVersion("package.json");
setJsonVersion("package-lock.json", (j) => {
  if (j.packages?.[""]) j.packages[""].version = next;
});
edit("src-tauri/Cargo.toml", (t) => t.replace(/^version = ".*?"/m, `version = "${next}"`));
edit("src-tauri/Cargo.lock", (t) =>
  t.replace(/(name = "halo-lyrics"\r?\nversion = )".*?"/, `$1"${next}"`),
);

console.log(`\nGlowline ${current} → ${next}\n`);
run("git add src-tauri/tauri.conf.json package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock");
run(`git commit -m "Release v${next}"`);
run("git push origin main");
console.log(`
✔ Pushed v${next}. GitHub Actions is building the update now (about 10–15 minutes).
  Watch it:  gh run watch
  When it finishes, installed copies update themselves (Settings → Check for updates to get it right away).
`);
