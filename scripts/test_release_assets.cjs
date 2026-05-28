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

run("tray icon decoder wires the embedded PNG on every host OS (SPEC #2920)", () => {
  // SPEC #2920 Phase 4 + 5: the legacy `native_window_icon()` helper
  // attached the GWT icon to a `tao::Window` for the wry WebView GUI
  // path. The tray-resident process no longer constructs a Window —
  // `main.rs` decodes the same `assets/icons/icon.png` into RGBA via
  // `load_tray_icon_rgba()` and hands it to `tray_icon::Icon::from_rgba`.
  // Pin both the decoder helper and the asset import so a regression
  // (renaming the helper or dropping the include_bytes!) is caught at
  // CI time.
  const mainRs = fs.readFileSync(
    path.join(__dirname, "..", "crates", "gwt", "src", "main.rs"),
    "utf8"
  );

  assert.match(
    mainRs,
    /fn load_tray_icon_rgba\(\) -> Option<\(Vec<u8>, u32, u32\)>/,
    "main.rs must expose load_tray_icon_rgba() so the tray icon decodes the embedded PNG cross-platform"
  );
  assert.match(
    mainRs,
    /include_bytes!\("\.\.\/\.\.\/\.\.\/assets\/icons\/icon\.png"\)/,
    "load_tray_icon_rgba must embed assets/icons/icon.png at compile time"
  );
  assert.match(
    mainRs,
    /tray_icon::Icon::from_rgba/,
    "main.rs must hand the decoded RGBA bytes to tray_icon::Icon::from_rgba"
  );
});

run("macOS app bundle declares and ships the CFBundleIconFile resource", () => {
  const workflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "release.yml"),
    "utf8"
  );
  assert.match(
    workflow,
    /<key>CFBundleIconFile<\/key>\s*\n\s*<string>icon<\/string>/,
    "build-dmg Info.plist must declare CFBundleIconFile=icon so macOS resolves the bundle icon"
  );
  assert.match(
    workflow,
    /mkdir -p dist\/GWT\.app\/Contents\/Resources/,
    "build-dmg must create Contents/Resources before copying icon.icns"
  );
  assert.match(
    workflow,
    /cp assets\/icons\/icon\.icns dist\/GWT\.app\/Contents\/Resources\/icon\.icns/,
    "build-dmg must copy assets/icons/icon.icns into Contents/Resources/icon.icns"
  );
});

run("macOS app bundle hides the Dock icon via LSUIElement=true (SPEC #2920)", () => {
  // SPEC #2920 FR-008: the tray-resident process must not show in the
  // macOS Dock or Cmd-Tab switcher. build-dmg owns the Info.plist
  // template, so the canonical assertion lives next to the bundle
  // resource checks above.
  const workflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "release.yml"),
    "utf8"
  );
  assert.match(
    workflow,
    /<key>LSUIElement<\/key>\s*\n\s*<true\/>/,
    "build-dmg Info.plist must declare LSUIElement=true so the tray-resident process stays out of the Dock"
  );
});

run("Linux distribution ships a gwt.desktop shortcut (SPEC #2920)", () => {
  // SPEC #2920 FR-009: Linux distribution must include a .desktop file
  // that the user can drop into ~/.local/share/applications/ for the
  // application menu and into ~/.config/autostart/ via the Settings
  // page. The canonical template lives under dist/.
  const desktopPath = path.join(__dirname, "..", "dist", "gwt.desktop");
  assert.ok(
    fs.existsSync(desktopPath),
    "dist/gwt.desktop must exist so Linux portable archive can ship a Desktop Entry"
  );
  const contents = fs.readFileSync(desktopPath, "utf8");
  for (const requiredLine of [
    /^\[Desktop Entry\]/m,
    /^Type=Application/m,
    /^Name=GWT/m,
    /^Exec=gwt/m,
    /^Icon=gwt/m,
    /^Categories=.*Development.*/m,
    /^StartupNotify=false/m,
  ]) {
    assert.match(
      contents,
      requiredLine,
      `dist/gwt.desktop must include ${requiredLine}`
    );
  }
});

run("repository does not expose gwt through npm package metadata", () => {
  for (const relativePath of [
    "package.json",
    "pnpm-lock.yaml",
    "scripts/postinstall.cjs",
    "bin/gwt.cjs",
    "bin/gwtd.cjs",
    "bin/launcher.cjs",
  ]) {
    assert.equal(
      fs.existsSync(path.join(__dirname, "..", relativePath)),
      false,
      `${relativePath} must not exist after npm distribution removal`
    );
  }
});

run("frontend bundle check keeps the GUI front door explicit", () => {
  const bundleScript = fs.readFileSync(
    path.join(__dirname, "check-frontend-bundle.sh"),
    "utf8"
  );
  // SPEC-2356 — the frontend bundle now also covers Operator Design System
  // ESM modules (theme-manager / hotkey / operator-shell). The contract is
  // expressed as required substrings so future modules can extend the chain
  // without rewriting the test, while still keeping the legacy SPEC-2008
  // surfaces (app, branch-cleanup-modal, migration-modal) wired up.
  for (const required of [
    "node --check crates/gwt/web/app.js",
    "node --check crates/gwt/web/branch-cleanup-modal.js",
    "node --check crates/gwt/web/migration-modal.js",
    "node --check crates/gwt/web/theme-manager.js",
    "node --check crates/gwt/web/theme-toggle.js",
    "node --check crates/gwt/web/hotkey.js",
    "node --check crates/gwt/web/operator-shell.js",
    "node --check crates/gwt/web/focus-trap.js",
  ]) {
    assert.ok(
      bundleScript.includes(required),
      `check-frontend-bundle.sh must include "${required}"`
    );
  }
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
  assert.match(releaseFlow, /scripts\/check-frontend-bundle\.sh/);
  assert.match(releaseFlow, /script type=\"module\" src=\"\/app\.js\"/);
});

run("CI workflows call direct verification scripts and skip npm publish", () => {
  const lintWorkflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "lint.yml"),
    "utf8"
  );
  const testWorkflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "test.yml"),
    "utf8"
  );
  const releaseWorkflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "release.yml"),
    "utf8"
  );
  const smokeTests = fs.readFileSync(
    path.join(__dirname, "run-frontend-smoke-tests.sh"),
    "utf8"
  );
  const unitTests = fs.readFileSync(
    path.join(__dirname, "run-frontend-unit-tests.sh"),
    "utf8"
  );
  assert.match(lintWorkflow, /bash scripts\/check-frontend-bundle\.sh/);
  assert.match(lintWorkflow, /bash scripts\/check-release-flow\.sh/);
  assert.match(testWorkflow, /node scripts\/test_release_assets\.cjs/);
  assert.doesNotMatch(releaseWorkflow, /publish-npm|npm publish|registry\.npmjs\.org|NPM_TOKEN/);
  for (const content of [smokeTests, unitTests]) {
    assert.doesNotMatch(content, /pnpm(?:\/action-setup| dlx|\b)/);
  }
});

run("README install guidance points to GUI-first release assets", () => {
  const readme = fs.readFileSync(path.join(__dirname, "..", "README.md"), "utf8");
  const readmeJa = fs.readFileSync(path.join(__dirname, "..", "README.ja.md"), "utf8");

  for (const doc of [readme, readmeJa]) {
    assert.match(doc, /gwt-macos-universal\.dmg/);
    assert.match(doc, /gwt-windows-x86_64\.msi/);
    assert.match(doc, /gwt-linux-x86_64\.tar\.gz/);
    assert.match(doc, /bash scripts\/check-frontend-bundle\.sh/);
    assert.match(doc, /bash scripts\/check-release-flow\.sh/);
  }
});

run("windows MSI diagnostics are documented for launch failures", () => {
  const readme = fs.readFileSync(path.join(__dirname, "..", "README.md"), "utf8");
  const readmeJa = fs.readFileSync(path.join(__dirname, "..", "README.ja.md"), "utf8");

  for (const doc of [readme, readmeJa]) {
    assert.match(doc, /diagnose-windows-msi\.ps1/);
    assert.match(doc, /msiexec|Windows Installer/);
  }
});

run("windows MSI diagnostic script captures installer evidence", () => {
  const diagnosticScript = fs.readFileSync(
    path.join(__dirname, "diagnose-windows-msi.ps1"),
    "utf8"
  );

  for (const required of [
    "Get-FileHash",
    "Get-AuthenticodeSignature",
    "Zone.Identifier",
    "Start-Transcript",
    "msiexec.exe",
    "/L*V",
    "Get-ChildItem",
    "gwt.exe",
    "--version",
    "serve",
    "--no-open",
  ]) {
    assert.ok(
      diagnosticScript.includes(required),
      `diagnose-windows-msi.ps1 must include "${required}"`
    );
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
