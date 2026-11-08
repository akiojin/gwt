import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
const workspace=process.cwd(),relative='docs/perf/issue-5379';
const original=path.join(workspace,relative),provenance=JSON.parse(fs.readFileSync(path.join(original,'provenance.json'),'utf8'));
const fixtureRoot=fs.mkdtempSync(path.join(workspace,'target/issue-5379-evidence-contract-'));
const fixtureDocs=path.join(fixtureRoot,relative);
for(const file of Object.keys(provenance.treatment_source_sha256)){
 const destination=path.join(fixtureRoot,file);fs.mkdirSync(path.dirname(destination),{recursive:true});fs.copyFileSync(path.join(workspace,file),destination);
}
const results=[];
function restore(){fs.cpSync(original,fixtureDocs,{recursive:true})}
function run(name,status,expected){
 const result=spawnSync(process.execPath,[path.join(fixtureDocs,'verify.mjs')],{cwd:fixtureRoot,encoding:'utf8'});
 assert.equal(result.status,status,name+' exit');if(expected)assert.match(result.stderr,expected,name+' reason');
 results.push({name,expected_exit:status,actual_exit:result.status,stdout:result.stdout,stderr:result.stderr});
}
function changeEnvironment(change){
 const file=path.join(fixtureDocs,'after-environment.json'),environment=JSON.parse(fs.readFileSync(file,'utf8'));change(environment);
 const bytes=JSON.stringify(environment,null,2)+'\n';fs.writeFileSync(file,bytes);
 const provenanceFile=path.join(fixtureDocs,'provenance.json'),current=JSON.parse(fs.readFileSync(provenanceFile,'utf8'));
 current.latency_acceptance.environment_sha256=createHash('sha256').update(bytes).digest('hex');fs.writeFileSync(provenanceFile,JSON.stringify(current,null,2)+'\n');
}
restore();run('approved pending records retain integrity',0);
restore();{
 const file=path.join(fixtureDocs,'provenance.json'),current=JSON.parse(fs.readFileSync(file,'utf8'));delete current.latency_acceptance;fs.writeFileSync(file,JSON.stringify(current,null,2)+'\n');
 run('missing disposition remains strict',1,/1 !== 0|after did not pass/);
}
restore();changeEnvironment(environment=>{environment.job_observations[0].target_started_at_utc=environment.job_observations[0].target_started_at_utc.replace(/(\d)Z$/,(_,digit)=>String((Number(digit)+1)%10)+'Z')});
run('exact GUI start mismatch is rejected',1,/cap proof start differs/);
restore();changeEnvironment(environment=>{assert.ok(environment.measurement_failures.length);environment.measurement_failures[0].message='Timeout: close_window cleanup failed'});
run('non-latency runner error is rejected',1,/non-latency runner error cannot be deferred/);
restore();{
 const file=path.join(fixtureDocs,'after-dark.json'),current=JSON.parse(fs.readFileSync(file,'utf8'));current.wire[0].sequences.pop();fs.writeFileSync(file,JSON.stringify(current,null,2)+'\n');
 run('missing functional output sequence is rejected',1,/missing sequence/);
}
const report={passed:true,fixture_root:fixtureRoot,cases:results};
fs.writeFileSync(path.join(fixtureRoot,'checker-contract-results.json'),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({passed:true,fixture_root:fixtureRoot,cases:results.map(({name,expected_exit,actual_exit})=>({name,expected_exit,actual_exit}))},null,2));