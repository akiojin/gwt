const assert = require("node:assert/strict");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");

const {
  binaryNameForPlatform,
  bundleBinaryNamesForPlatform,
  daemonBinaryNameForPlatform,
  installBundleFromArchive,
  primaryReleaseAssetName,
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

run("primary release asset names match the GUI-first install contract", () => {
  assert.equal(primaryReleaseAssetName("darwin", "arm64"), "gwt-macos-universal.dmg");
  assert.equal(primaryReleaseAssetName("darwin", "x64"), "gwt-macos-universal.dmg");
  assert.equal(primaryReleaseAssetName("linux", "arm64"), "gwt-linux-aarch64.tar.gz");
  assert.equal(primaryReleaseAssetName("linux", "x64"), "gwt-linux-x86_64.tar.gz");
  assert.equal(primaryReleaseAssetName("win32", "x64"), "gwt-windows-x86_64.msi");
});

run("release helper keeps platform binary names stable", () => {
  assert.equal(binaryNameForPlatform("win32"), "gwt.exe");
  assert.equal(binaryNameForPlatform("linux"), "gwt");
  assert.equal(binaryNameForPlatform("darwin"), "gwt");
  assert.equal(daemonBinaryNameForPlatform("win32"), "gwtd.exe");
  assert.equal(daemonBinaryNameForPlatform("linux"), "gwtd");
  assert.equal(daemonBinaryNameForPlatform("darwin"), "gwtd");
});

run("release helper keeps bundle binary names stable", () => {
  assert.deepEqual(bundleBinaryNamesForPlatform("win32"), ["gwt.exe", "gwtd.exe"]);
  assert.deepEqual(bundleBinaryNamesForPlatform("linux"), ["gwt", "gwtd"]);
  assert.deepEqual(bundleBinaryNamesForPlatform("darwin"), ["gwt", "gwtd"]);
});

run("installer entrypoints are loadable under package type module", () => {
  const daemonLauncher = require("../bin/gwtd.cjs");

  assert.equal(typeof postinstall.main, "function");
  assert.equal(typeof launcher.main, "function");
  assert.equal(typeof daemonLauncher.main, "function");
  assert.equal(typeof daemonLauncher.readVersion, "function");
});

run("windows installer definition includes the gwtd companion binary", () => {
  const wix = fs.readFileSync(path.join(__dirname, "..", "wix", "main.wxs"), "utf8");
  assert.match(wix, /gwtd\.exe/);
});

run("release workflow packages gwtd alongside gwt", () => {
  const workflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "release.yml"),
    "utf8"
  );
  assert.match(workflow, /--bin gwt --bin gwtd/);
  assert.match(workflow, /Compress-Archive -Path @\("dist\/gwt\.exe", "dist\/gwtd\.exe"\)/);
  assert.match(workflow, /tar -czf \$\{\{ matrix\.archive_name \}\} gwt gwtd/);
  assert.match(workflow, /Contents\/MacOS\/gwtd/);
});

run("package scripts keep the GUI front door and release contract explicit", () => {
  const pkg = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "package.json"), "utf8"));
  assert.equal(pkg.bin.gwt, "bin/gwt.cjs");
  assert.equal(pkg.bin.gwtd, "bin/gwtd.cjs");
  assert.equal(pkg.scripts["test:release-assets"], "node scripts/test_release_assets.cjs");
  assert.equal(
    pkg.scripts["test:frontend-bundle"],
    "node --check crates/gwt/web/app.js && node --check crates/gwt/web/branch-cleanup-modal.js && node --check crates/gwt/web/migration-modal.js"
  );
  assert.equal(pkg.scripts["test:release-flow"], "bash scripts/check-release-flow.sh");
  assert.equal(pkg.scripts.dev, "cargo run -p gwt --bin gwt");
  assert.equal(pkg.scripts.build, "cargo build --release -p gwt --bin gwt --bin gwtd");
});

run("macos install scripts install and remove both public command shims", () => {
  const install = fs.readFileSync(
    path.join(__dirname, "..", "installers", "macos", "install.sh"),
    "utf8"
  );
  const uninstall = fs.readFileSync(
    path.join(__dirname, "..", "installers", "macos", "uninstall.sh"),
    "utf8"
  );

  assert.match(install, /gwt-macos-\$\{ARCH\}\.tar\.gz/);
  assert.match(install, /for BIN in gwt gwtd/);
  assert.match(install, /chmod \+x "\$INSTALL_DIR\/\$BIN"/);
  assert.match(uninstall, /for BIN in gwt gwtd/);
  assert.match(uninstall, /rm -f "\$INSTALL_DIR\/\$BIN"/);
});

run("test all script keeps rust tests plus frontend release checks", () => {
  const testAll = fs.readFileSync(path.join(__dirname, "test-all.sh"), "utf8");
  assert.match(testAll, /test -p gwt-core -p gwt --all-features/);
  assert.match(testAll, /cargo\.exe/);
  assert.match(testAll, /bash scripts\/check-release-flow\.sh/);
});

run("release flow helper checks the shared frontend bundle and release assets", () => {
  const releaseFlow = fs.readFileSync(path.join(__dirname, "check-release-flow.sh"), "utf8");
  assert.match(releaseFlow, /scripts\/test_release_assets\.cjs/);
  assert.match(releaseFlow, /node --check \"\$APP_JS\"/);
  assert.match(releaseFlow, /script type=\"module\" src=\"\/app\.js\"/);
});

run("CI workflows call the named frontend and release verification scripts", () => {
  const lintWorkflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "lint.yml"),
    "utf8"
  );
  const testWorkflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "test.yml"),
    "utf8"
  );

  assert.match(lintWorkflow, /test:frontend-bundle/);
  assert.match(lintWorkflow, /test:release-flow/);
  assert.match(testWorkflow, /test:release-assets/);
});

run("README install guidance points to GUI-first release assets", () => {
  const readme = fs.readFileSync(path.join(__dirname, "..", "README.md"), "utf8");
  const readmeJa = fs.readFileSync(path.join(__dirname, "..", "README.ja.md"), "utf8");

  for (const doc of [readme, readmeJa]) {
    assert.match(doc, /gwt-macos-universal\.dmg/);
    assert.match(doc, /gwt-windows-x86_64\.msi/);
    assert.match(doc, /gwt-linux-x86_64\.tar\.gz/);
    assert.match(doc, /test:frontend-bundle|node --check crates\/gwt\/web\/app\.js/);
    assert.match(doc, /test:release-flow|bash scripts\/check-release-flow\.sh/);
  }
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
  if (process.platform !== "win32") {
    return;
  }

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
