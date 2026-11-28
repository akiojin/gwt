// Recompute the P10 nearest-rank quantiles from the committed raw scan samples
// and require the README table to state the same values (SPEC #3820 AC-2 evidence).
import { readFileSync } from "node:fs";
const dir = "docs/perf/issue-3820";
const readme = readFileSync(`${dir}/README.md`, "utf8");
const fmt = value => value.toFixed(1);
const quantiles = values => {
  const sorted = [...values].sort((a, b) => a - b);
  const rank = q => sorted[Math.max(0, Math.ceil(q * sorted.length) - 1)];
  return [rank(0.5), rank(0.95), sorted.at(-1)];
};
let failures = 0;
const rows = [];
for (const [theme, label] of [["dark", "Dark"], ["light", "Light"]]) {
  const run = JSON.parse(readFileSync(`${dir}/scan-chromium-${theme}.json`, "utf8"));
  const samples = run.P10.samples_ms;
  if (samples.length !== 10 || run.console_errors.length || run.P10.launch_decision_ms.length) {
    console.error(`P10 ${theme}: expected 10 clean samples and no launch decision`); failures += 1;
  }
  rows.push([label, samples]);
}
rows.push(["合算", rows.flatMap(([, samples]) => samples)]);
for (const [label, samples] of rows) {
  const row = `| ${label} | ${samples.length} | ${quantiles(samples).map(fmt).join(" | ")} |`;
  if (!readme.includes(row)) { console.error(`README is missing: ${row}`); failures += 1; }
}
if (failures) process.exit(1);
console.log("P10 scan evidence matches the README");
