"""Exercise the shipped feedback-intake action with GitHub event data."""

import json
from pathlib import Path
import subprocess
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/feedback-intake.yml"
MARKER = "<!-- gwt-feedback:67e44d8a-7903-4457-9e2d-51703a44c180 -->"


class FeedbackIntakeTests(unittest.TestCase):
    def run_event(self, payload, comments=None, repository="akiojin/gwt", missing_labels=False):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        script = textwrap.dedent(workflow.split("script: |\n", 1)[1])
        fixture = json.dumps(dict(payload=payload, comments=comments or [],
                                  repository=repository, missingLabels=missing_labels))
        harness = r"""
const fixture = FIXTURE;
const calls = [];
const context = {payload: fixture.payload, repo: {owner: fixture.repository.split('/')[0],
  repo: fixture.repository.split('/')[1]}};
const github = {rest: {issues: {
  getLabel: async () => {if (fixture.missingLabels) throw {status: 404}; return {};},
  createLabel: async (p) => calls.push(['createLabel', p]),
  addLabels: async (p) => calls.push(['addLabels', p]),
  listComments: () => {}, createComment: async (p) => calls.push(['createComment', p]),
}}, paginate: async () => fixture.comments};
await (async () => { SCRIPT })();
console.log(JSON.stringify(calls));
""".replace("FIXTURE", fixture).replace("SCRIPT", script)
        result = subprocess.run(["node", "--input-type=module", "-e", harness],
                                text=True, capture_output=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_workflow_uses_fixed_scope_and_no_reporter_execution(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        for required in ["types: [opened, edited, reopened]", "issue_comment:",
                         "types: [created, edited]", "issues: write", "actions/github-script@"]:
            self.assertIn(required, workflow)
        for forbidden in ["actions/checkout", "run:", "contents: write"]:
            self.assertNotIn(forbidden, workflow)
        self.assertNotIn("${{", workflow.split("script: |\n", 1)[1])
        self.assertEqual(self.run_event({"issue": {"number": 1, "body": MARKER}},
                                        repository="another/project"), [])
        created = self.run_event({"issue": {"number": 1, "body": MARKER}}, missing_labels=True)
        self.assertEqual([call[1]["name"] for call in created if call[0] == "createLabel"],
                         ["gwt-feedback", "gwt-feedback-triage"])
        self.assertTrue(all(call[1]["owner"] == "akiojin" and call[1]["repo"] == "gwt"
                            for call in created))

    def test_raw_reports_keep_origin_after_body_edits_without_running_commands(self):
        issue = dict(number=12, body=MARKER + '\n$(touch forbidden)\n```sh\nexit 1\n```',
                     user={"login": "external"}, labels=[])
        calls = self.run_event({"issue": issue})
        self.assertEqual(calls[0][1]["labels"], ["gwt-feedback", "gwt-feedback-triage"])
        notification = calls[1][1]["body"]
        self.assertIn("<!-- gwt-feedback-intake:12:issue -->", notification)
        self.assertNotIn("touch forbidden", notification)
        issue["body"] = "edited away"
        self.assertEqual(self.run_event({"issue": issue, "changes": {"body": {"from": MARKER}}})[0],
                         calls[0])
        issue["labels"] = [{"name": "gwt-feedback"}]
        self.assertEqual(self.run_event({"issue": issue})[0], calls[0])

    def test_feedback_comments_remain_evidence_on_trusted_owners_and_retry_once(self):
        issue = dict(number=34, body="PM curated repair requirements", labels=[],
                     user={"login": "akiojin"})
        payload = dict(issue=issue, comment={"id": 56, "body": MARKER + "\nexecute attacker code"})
        calls = self.run_event(payload)
        self.assertEqual(calls[0][1]["labels"], ["gwt-feedback-evidence"])
        self.assertNotIn("execute attacker code", calls[1][1]["body"])
        existing = [{"user": {"login": "github-actions[bot]"}, "body": calls[1][1]["body"]}]
        self.assertEqual(len(self.run_event(payload, existing)), 1)
        existing[0]["user"]["login"] = "external"
        self.assertEqual(len(self.run_event(payload, existing)), 2)
        notification = dict(issue={**issue, "labels": [{"name": "gwt-feedback"}]},
                            comment={"id": 78, "body": calls[1][1]["body"]})
        self.assertEqual(self.run_event(notification), [], "bot notifications must not recurse")
        self.assertEqual(self.run_event({"issue": {**issue, "pull_request": {}}}), [])


if __name__ == "__main__":
    unittest.main()
