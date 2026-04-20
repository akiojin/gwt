const assert = require("node:assert/strict");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");

const {
  binaryNameForPlatform,
  installBinaryFromArchive,
  releaseAssetName,
} = require("./release-assets.cjs");
const postinstall = require("./postinstall.cjs");
const launcher = require("../bin/gwt.cjs");

let failed = false;

function run(name, fn) {
  try {
    fn();
    console.log(`ok - ${name}`);
  } catch (error) {
    failed = true;
    console.error(`not ok - ${name}`);
    console.error(error && error.stack ? error.stack : error);
  }
}

run("release asset names match the public portable contract", () => {
  assert.equal(releaseAssetName("darwin", "arm64"), "gwt-macos-arm64.tar.gz");
  assert.equal(releaseAssetName("darwin", "x64"), "gwt-macos-x86_64.tar.gz");
  assert.equal(releaseAssetName("linux", "arm64"), "gwt-linux-aarch64.tar.gz");
  assert.equal(releaseAssetName("linux", "x64"), "gwt-linux-x86_64.tar.gz");
  assert.equal(releaseAssetName("win32", "x64"), "gwt-windows-x86_64.zip");
});

run("release helper keeps platform binary names stable", () => {
  assert.equal(binaryNameForPlatform("win32"), "gwt.exe");
  assert.equal(binaryNameForPlatform("linux"), "gwt");
  assert.equal(binaryNameForPlatform("darwin"), "gwt");
});

run("installer entrypoints are loadable under package type module", () => {
  assert.equal(typeof postinstall.main, "function");
  assert.equal(typeof launcher.main, "function");
});

run("portable tarball extraction installs the unix binary", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-release-test-"));
  const sourceDir = path.join(root, "source");
  const binDir = path.join(root, "bin");
  const archivePath = path.join(root, "gwt-linux-x86_64.tar.gz");
  fs.mkdirSync(sourceDir, { recursive: true });
  fs.writeFileSync(path.join(sourceDir, "gwt"), "unix-binary");

  execFileSync("tar", ["-czf", archivePath, "-C", sourceDir, "gwt"]);

  installBinaryFromArchive({
    archivePath,
    asset: path.basename(archivePath),
    binDir,
    binaryName: "gwt",
    platform: "linux",
  });

  assert.equal(fs.readFileSync(path.join(binDir, "gwt"), "utf8"), "unix-binary");
});

run("portable zip extraction installs the windows binary", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-release-test-"));
  const sourceDir = path.join(root, "source");
  const binDir = path.join(root, "bin");
  const archivePath = path.join(root, "gwt-windows-x86_64.zip");
  const sourceBinary = path.join(sourceDir, "gwt.exe");
  fs.mkdirSync(sourceDir, { recursive: true });
  fs.writeFileSync(sourceBinary, "windows-binary");

  execFileSync("powershell.exe", [
    "-NoProfile",
    "-NonInteractive",
    "-Command",
    `Compress-Archive -LiteralPath '${sourceBinary.replace(/'/g, "''")}' -DestinationPath '${archivePath.replace(/'/g, "''")}' -Force`,
  ]);

  installBinaryFromArchive({
    archivePath,
    asset: path.basename(archivePath),
    binDir,
    binaryName: "gwt.exe",
    platform: "win32",
  });

  assert.equal(fs.readFileSync(path.join(binDir, "gwt.exe"), "utf8"), "windows-binary");
});

if (failed) {
  process.exit(1);
}
