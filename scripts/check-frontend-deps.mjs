#!/usr/bin/env node
// SPEC #6943 AC-9: check every locked license (including dev/optional packages),
// then let npm classify production advisories. Registry failure is not a pass.
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { fileURLToPath } from "node:url";

const repo = fileURLToPath(new URL("..", import.meta.url));
const allowed = new Set(["MIT", "ISC", "BSD-2-Clause", "BSD-3-Clause", "Apache-2.0", "0BSD"]);
const recovery = "Recovery: update or replace the dependency, regenerate and commit the lockfile/bundle; "
  + "review license exceptions in scripts/frontend-license-exceptions.json with an exact package/version/license and reason. "
  + "For advisories or registry errors, run npm audit --prefix crates/gwt/web-react --omit=dev --audit-level=high and retry this gate.";

try {
  const { values } = parseArgs({ options: {
    "package-dir": { type: "string", default: join(repo, "crates/gwt/web-react") },
    exceptions: { type: "string", default: join(repo, "scripts/frontend-license-exceptions.json") },
  } });
  const lock = JSON.parse(readFileSync(join(values["package-dir"], "package-lock.json"), "utf8"));
  const exceptions = JSON.parse(readFileSync(values.exceptions, "utf8"));
  if (![2, 3].includes(lock.lockfileVersion) || !lock.packages || typeof lock.packages !== "object" || Array.isArray(lock.packages)) {
    throw new Error("expected a package-lock v2/v3 packages object; regenerate it with npm install --package-lock-only");
  }
  if (!Array.isArray(exceptions) || exceptions.some(entry => !entry
    || ["package", "version", "license", "reason"].some(key => typeof entry[key] !== "string" || !entry[key].trim()))) {
    throw new Error("each license exception requires a nonempty package, version, license and review reason");
  }

  const packages = Object.entries(lock.packages).filter(([path]) => path !== "");
  const violations = [];
  for (const [path, pkg] of packages) {
    const name = pkg?.name || path.split("node_modules/").at(-1);
    const version = pkg?.version || "UNKNOWN";
    const license = pkg?.license || "UNKNOWN";
    if (!allowed.has(license) && !exceptions.some(entry => entry.package === name && entry.version === version && entry.license === license)) {
      violations.push(`${name}@${version}: ${JSON.stringify(license)} (${path})`);
    }
  }
  if (violations.length) throw new Error(`disallowed/unknown licenses:\n${violations.join("\n")}`);
  console.log(`frontend licenses: ${packages.length} locked packages accepted (including dev/optional dependencies)`);

  const audit = spawnSync("npm", ["audit", "--omit=dev", "--audit-level=high", "--json"], {
    cwd: values["package-dir"], encoding: "utf8", timeout: 120_000, maxBuffer: 8 * 1024 * 1024,
  });
  let report;
  try { report = JSON.parse(audit.stdout); } catch { /* Handled as unknown below. */ }
  const counts = report?.metadata?.vulnerabilities;
  if (audit.error || audit.signal || report?.error || !counts
    || ![counts.high, counts.critical].every(count => Number.isInteger(count) && count >= 0)) {
    throw new Error(`npm audit outcome unknown: ${audit.error?.message || audit.signal || audit.stdout || audit.stderr || "missing vulnerability counts"}`);
  }
  if (counts.high > 0 || counts.critical > 0) {
    throw new Error(`npm audit found high/critical vulnerabilities:\n${audit.stdout}`);
  }
  if (audit.status !== 0) throw new Error(`npm audit outcome unknown (exit ${audit.status}): ${audit.stdout}\n${audit.stderr}`);
  console.log("frontend audit: no high/critical production vulnerabilities");
} catch (error) {
  console.error(`frontend dependencies: ${error.message}\n${recovery}`);
  process.exitCode = 1;
}
