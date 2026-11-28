// Recompute nearest-rank quantiles from the committed raw samples and require
// the README tables to state the same values (SPEC #3820 AC-2 evidence).
import { readFileSync } from "node:fs";
const dir = "docs/perf/issue-3820";
const readme = readFileSync(`${dir}/README.md`, "utf8");
const runs = ["dark", "light"].map(theme => JSON.parse(readFileSync(`${dir}/terminal-${theme}.json`, "utf8")));
const fmt = value => value.toFixed(1);
const quantiles = values => {
  const sorted = [...values].sort((a, b) => a - b);
  const rank = q => sorted[Math.max(0, Math.ceil(q * sorted.length) - 1)];
  return [rank(0.5), rank(0.95), sorted.at(-1)];
};
let failures = 0;
for (const path of ["P07", "P17", "P18"]) {
  for (const run of runs) {
    if (run[path].samples_ms.length !== 30 || run.console_errors.length || run.delivery_errors.length) {
      console.error(`${path} ${run.theme}: expected 30 clean samples`); failures += 1;
    }
  }
  const [p50, p95, worst] = quantiles(runs.flatMap(run => run[path].samples_ms));
  const row = new RegExp(`\\| ${path} \\| [^|]+ \\| 60 \\| ${fmt(p50)} \\| ${fmt(p95)} \\| ${fmt(worst)} \\|`);
  if (!row.test(readme)) { console.error(`${path}: README row does not match ${fmt(p50)} / ${fmt(p95)} / ${fmt(worst)}`); failures += 1; }
  const perTheme = runs.map(run => quantiles(run[path].samples_ms).map(fmt).join(" / ")).join(" | ");
  if (!readme.includes(`| ${path} | ${perTheme} |`)) { console.error(`${path}: per-theme row mismatch (${perTheme})`); failures += 1; }
}
if (failures) process.exit(1);
console.log("terminal latency evidence matches raw samples");
