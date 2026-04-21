const assert = require("node:assert/strict");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");

const {
  binaryNameForPlatform,
  bundleBinaryNamesForPlatform,
  installBundleFromArchive,
  readReleaseContract,
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
  const contract = readReleaseContract();
  assert.equal(releaseAssetName("darwin", "arm64"), contract.portable_assets["macos-aarch64"]);
  assert.equal(releaseAssetName("darwin", "x64"), contract.portable_assets["macos-x86_64"]);
  assert.equal(releaseAssetName("linux", "arm64"), contract.portable_assets["linux-aarch64"]);
  assert.equal(releaseAssetName("linux", "x64"), contract.portable_assets["linux-x86_64"]);
  assert.equal(releaseAssetName("win32", "x64"), contract.portable_assets["windows-x86_64"]);
});

run("release asset helper reads the shared contract", () => {
  const contract = readReleaseContract();
  assert.equal(contract.portable_assets["windows-x86_64"], "gwt-windows-x86_64.zip");
  assert.equal(contract.installer_assets.windows, "gwt-windows-x86_64.msi");
  assert.deepEqual(contract.bundle_binaries.windows, ["gwt.exe", "gwtd.exe"]);
});

run("release helper keeps platform binary names stable", () => {
  assert.equal(binaryNameForPlatform("win32"), "gwt.exe");
  assert.equal(binaryNameForPlatform("linux"), "gwt");
  assert.equal(binaryNameForPlatform("darwin"), "gwt");
});

run("release helper keeps bundle binary names stable", () => {
  const contract = readReleaseContract();
  assert.deepEqual(bundleBinaryNamesForPlatform("win32"), contract.bundle_binaries.windows);
  assert.deepEqual(bundleBinaryNamesForPlatform("linux"), contract.bundle_binaries.linux);
  assert.deepEqual(bundleBinaryNamesForPlatform("darwin"), contract.bundle_binaries.macos);
});

run("installer entrypoints are loadable under package type module", () => {
  assert.equal(typeof postinstall.main, "function");
  assert.equal(typeof launcher.main, "function");
});

run("windows installer definition includes the gwtd companion binary", () => {
  const wix = fs.readFileSync(path.join(__dirname, "..", "wix", "main.wxs"), "utf8");
  assert.match(wix, /gwtd\.exe/);
});

run("windows icon assets are available for exe and installer branding", () => {
  for (const asset of ["icon.ico", "icon.png", "icon.icns"]) {
    const icon = path.join(__dirname, "..", "assets", "icons", asset);
    assert.ok(fs.statSync(icon).size > 0, `${asset} should be present`);
  }
});

run("windows installer is per-user and keeps updater migration on the MSI path", () => {
  const wix = fs.readFileSync(path.join(__dirname, "..", "wix", "main.wxs"), "utf8");
  assert.match(wix, /Scope="perUser"/);
  assert.match(wix, /LocalAppDataFolder/);
  assert.match(wix, /Environment[^>]+Name="PATH"[^>]+Part="last"/);
  assert.match(wix, /ProgramMenuFolder/);
  assert.match(wix, /Shortcut[^>]+Id="GwtStartMenuShortcut"[^>]+Name="GWT"/);
  assert.match(wix, /Icon[^>]+Id="GwtIcon\.exe"/);
  assert.match(wix, /GWT_LEGACY_PER_MACHINE_EXE/);
  assert.match(wix, /GWT_ALLOW_LEGACY_MIGRATION/);
  assert.match(
    wix,
    /Condition="NOT GWT_LEGACY_PER_MACHINE_EXE OR GWT_ALLOW_LEGACY_MIGRATION = &quot;1&quot;"/
  );
});

run("release workflow packages gwtd alongside gwt", () => {
  const contract = readReleaseContract();
  const workflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "release.yml"),
    "utf8"
  );
  for (const asset of Object.values(contract.portable_assets)) {
    assert.match(workflow, new RegExp(asset.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }
  for (const asset of Object.values(contract.installer_assets)) {
    assert.match(workflow, new RegExp(asset.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }
  assert.match(workflow, /--bin gwt --bin gwtd/);
  assert.match(workflow, /Compress-Archive -Path @\("dist\/gwt\.exe", "dist\/gwtd\.exe"\)/);
  assert.match(workflow, /tar -czf \$\{\{ matrix\.archive_name \}\} gwt gwtd/);
  assert.match(workflow, /Contents\/MacOS\/gwtd/);
});

run("portable tarball extraction installs the unix bundle", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-release-test-"));
  const sourceDir = path.join(root, "source");
  const binDir = path.join(root, "bin");
  const archivePath = path.join(root, "gwt-linux-x86_64.tar.gz");
  fs.mkdirSync(sourceDir, { recursive: true });
  fs.writeFileSync(path.join(sourceDir, "gwt"), "unix-binary");
  fs.writeFileSync(path.join(sourceDir, "gwtd"), "unix-daemon");

  execFileSync("tar", ["-czf", archivePath, "-C", sourceDir, "gwt", "gwtd"]);

  installBundleFromArchive({
    archivePath,
    asset: path.basename(archivePath),
    binDir,
    platform: "linux",
  });

  assert.equal(fs.readFileSync(path.join(binDir, "gwt"), "utf8"), "unix-binary");
  assert.equal(fs.readFileSync(path.join(binDir, "gwtd"), "utf8"), "unix-daemon");
});

run("portable zip extraction installs the windows bundle", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-release-test-"));
  const sourceDir = path.join(root, "source");
  const binDir = path.join(root, "bin");
  const archivePath = path.join(root, "gwt-windows-x86_64.zip");
  const sourceBinary = path.join(sourceDir, "gwt.exe");
  const sourceDaemon = path.join(sourceDir, "gwtd.exe");
  fs.mkdirSync(sourceDir, { recursive: true });
  fs.writeFileSync(sourceBinary, "windows-binary");
  fs.writeFileSync(sourceDaemon, "windows-daemon");

  execFileSync("powershell.exe", [
    "-NoProfile",
    "-NonInteractive",
    "-Command",
    `Compress-Archive -LiteralPath @('${sourceBinary.replace(/'/g, "''")}','${sourceDaemon.replace(/'/g, "''")}') -DestinationPath '${archivePath.replace(/'/g, "''")}' -Force`,
  ]);

  installBundleFromArchive({
    archivePath,
    asset: path.basename(archivePath),
    binDir,
    platform: "win32",
  });

  assert.equal(fs.readFileSync(path.join(binDir, "gwt.exe"), "utf8"), "windows-binary");
  assert.equal(fs.readFileSync(path.join(binDir, "gwtd.exe"), "utf8"), "windows-daemon");
});

if (failed) {
  process.exit(1);
}
