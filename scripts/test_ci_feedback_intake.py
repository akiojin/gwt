"""Exercise the shipped feedback-intake action with GitHub event data."""

import json
from pathlib import Path
import subprocess
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/feedback-intake.yml"
MARKER = "<!-- gwt-feedback:67e44d8a-7903-4457-9e2d-51703a44c180 -->"
SECOND = "<!-- gwt-feedback:77e44d8a-7903-4457-9e2d-51703a44c180 -->"
THIRD = "<!-- gwt-feedback:87e44d8a-7903-4457-9e2d-51703a44c180 -->"


class FeedbackIntakeTests(unittest.TestCase):
    def run_event(self, payload, comments=None, repository="akiojin/gwt", missing_labels=False,
                  current_issue=None, pages=None, fail_page=None, expected_error=None):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        script = textwrap.dedent(workflow.split("script: |\n", 1)[1])
        fixture = json.dumps(dict(payload=payload, comments=comments or [],
                                  repository=repository, missingLabels=missing_labels,
                                  currentIssue=current_issue or payload.get("issue"),
                                  pages=pages, failPage=fail_page))
        harness = r"""
const fixture = FIXTURE;
const calls = [];
const context = {payload: fixture.payload, repo: {owner: fixture.repository.split('/')[0],
  repo: fixture.repository.split('/')[1]}};
const github = {rest: {issues: {
  get: async () => ({data: fixture.currentIssue}),
  getLabel: async () => {if (fixture.missingLabels) throw {status: 404}; return {};},
  createLabel: async (p) => calls.push(['createLabel', p]),
  addLabels: async (p) => calls.push(['addLabels', p]),
  listComments: () => {}, createComment: async (p) => calls.push(['createComment', p]),
  updateComment: async (p) => calls.push(['updateComment', p]),
}}, paginate: async () => {
  const result = [];
  for (const [index, page] of (fixture.pages || [fixture.comments]).entries()) {
    if (index === fixture.failPage) throw Error('incomplete comments');
    result.push(...page);
  }
  return result;
}};
let error = null;
try { await (async () => { SCRIPT })(); } catch (failure) { error = failure.message; }
console.log(JSON.stringify({calls, error}));
""".replace("FIXTURE", fixture).replace("SCRIPT", script)
        result = subprocess.run(["node", "--input-type=module", "-e", harness],
                                text=True, capture_output=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        response = json.loads(result.stdout)
        self.assertEqual(response["error"], expected_error)
        return response["calls"]

    def summary_writes(self, calls):
        return [call for call in calls if call[0] in ("createComment", "updateComment")
                and call[1]["body"].startswith("<!-- gwt-feedback-aggregation:")]

    def notifications(self, calls):
        return [call for call in calls if call[0] == "createComment"
                and call[1]["body"].startswith("<!-- gwt-feedback-intake:")]

    def test_workflow_uses_fixed_scope_and_no_reporter_execution(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        for required in ["types: [opened, edited, reopened]", "issue_comment:",
                         "types: [created, edited, deleted]", "issues: write", "actions/github-script@"]:
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
        notification = self.notifications(calls)[0][1]["body"]
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
        self.assertNotIn("execute attacker code", self.notifications(calls)[0][1]["body"])
        existing = [{"id": index, "user": {"login": "github-actions[bot]"}, "body": call[1]["body"]}
                    for index, call in enumerate(calls) if call[0] == "createComment"]
        self.assertEqual(len(self.run_event(payload, existing)), 1)
        for item in existing:
            item["user"]["login"] = "external"
        self.assertEqual(len(self.notifications(self.run_event(payload, existing))), 1)
        notification = dict(issue={**issue, "labels": [{"name": "gwt-feedback"}]},
                            comment={"id": 78, "body": self.notifications(calls)[0][1]["body"]})
        self.assertEqual(self.run_event(notification), [], "bot notifications must not recurse")
        notification["comment"]["body"] = "<!-- gwt-feedback-aggregation:34 -->\nDistinct retained public feedback reports: 1."
        self.assertEqual(self.run_event(notification), [], "summary notifications must not recurse")
        self.assertEqual(self.run_event({"issue": {**issue, "pull_request": {}}}), [])

    def test_aggregation_counts_distinct_leading_canonical_payloads_only(self):
        issue = {"number": 12, "body": MARKER, "labels": []}
        comments = [{"body": MARKER}, {"body": SECOND + "\nnew OS/version"},
                    {"body": "> " + THIRD}, {"body": "```text\n" + THIRD + "\n```"},
                    {"body": "quoted evidence\n" + THIRD},
                    {"body": "<!-- gwt-feedback:not-a-uuid -->"}]
        writes = self.summary_writes(self.run_event({"issue": issue}, comments))
        self.assertEqual(len(writes), 1)
        self.assertIn("Distinct retained public feedback reports: 2.", writes[0][1]["body"])
        self.assertIn("not verified incidents or affected users", writes[0][1]["body"])
        self.assertNotIn("new OS/version", writes[0][1]["body"])

    def test_aggregation_updates_only_a_trusted_bot_and_deduplicates_same_uuid_retry(self):
        issue = {"number": 12, "body": MARKER, "labels": []}
        marker = "<!-- gwt-feedback-aggregation:12 -->"
        forged = {"id": 5, "user": {"login": "external"}, "body": marker + "\ncount999"}
        trusted = {"id": 7, "user": {"login": "github-actions[bot]"}, "body": marker + "\nstale"}
        notice = {"user": {"login": "github-actions[bot]"}, "body": "<!-- gwt-feedback-intake:12:56 -->"}
        payload = {"issue": issue, "comment": {"id": 56, "body": MARKER}}
        comments = [forged, trusted, {"body": MARKER}, notice]
        writes = self.summary_writes(self.run_event(payload, comments))
        self.assertEqual([call[0] for call in writes], ["updateComment"])
        self.assertEqual(writes[0][1]["comment_id"], 7)
        self.assertIn("Distinct retained public feedback reports: 1.", writes[0][1]["body"])
        trusted["body"] = writes[0][1]["body"]
        self.assertEqual(self.summary_writes(self.run_event(payload, comments)), [])
        writes = self.summary_writes(self.run_event(payload, [forged, notice]))
        self.assertEqual([call[0] for call in writes], ["createComment"])

    def test_aggregation_reads_current_body_and_all_pages_before_mutation(self):
        issue = {"number": 12, "body": MARKER, "labels": []}
        pages = [[{"body": MARKER}], [{"body": THIRD}]]
        calls = self.run_event({"issue": issue}, current_issue={**issue, "body": SECOND}, pages=pages)
        summaries = self.summary_writes(calls)
        self.assertEqual(len(summaries), 1)
        self.assertIn("Distinct retained public feedback reports: 3.", summaries[0][1]["body"])
        self.assertEqual(self.run_event({"issue": issue}, pages=pages, fail_page=1,
                                       expected_error="incomplete comments"), [],
                         "partial pagination must not publish any update")

        owner = {"number": 34, "body": "PM curated repair requirements", "labels": []}
        summary = {"id": 7, "user": {"login": "github-actions[bot]"},
                   "body": "<!-- gwt-feedback-aggregation:34 -->\nstale"}
        notice = {"user": {"login": "github-actions[bot]"},
                  "body": "<!-- gwt-feedback-intake:34:56 -->"}
        edit = {"action": "edited", "issue": owner,
                "comment": {"id": 56, "body": "marker removed"},
                "changes": {"body": {"from": MARKER}}}
        calls = self.run_event(edit, pages=[[summary, notice], [{"body": SECOND}]])
        self.assertEqual(len(self.summary_writes(calls)), 1)
        self.assertIn("Distinct retained public feedback reports: 1.",
                      self.summary_writes(calls)[0][1]["body"])
        self.assertEqual(self.notifications(calls), [], "edited evidence keeps its prior notice")
        deleted = {"action": "deleted", "issue": owner,
                   "comment": {"id": 56, "body": MARKER}}
        calls = self.run_event(deleted, pages=[[summary, notice]])
        self.assertEqual(len(self.summary_writes(calls)), 1)
        self.assertIn("Distinct retained public feedback reports: 0.",
                      self.summary_writes(calls)[0][1]["body"])
        self.assertEqual(self.notifications(calls), [], "deleted evidence keeps its prior notice")
        self.assertTrue(all(call[1]["labels"] == ["gwt-feedback-evidence"]
                            for call in calls if call[0] == "addLabels"))


if __name__ == "__main__":
    unittest.main()
