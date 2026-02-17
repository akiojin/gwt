#!/usr/bin/env python3
"""gwt issue-first spec MCP server.

Provides GitHub Issue based Spec Kit tools over stdio MCP.
"""

from __future__ import annotations

import json
import subprocess
import sys
from dataclasses import dataclass
from typing import Any, Dict, Optional


TOOLS = [
    {
        "name": "spec_issue_upsert",
        "description": "Create or update issue-first spec sections by SPEC ID.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "spec_id": {"type": "string"},
                "title": {"type": "string"},
                "sections": {
                    "type": "object",
                    "properties": {
                        "spec": {"type": "string"},
                        "plan": {"type": "string"},
                        "tasks": {"type": "string"},
                        "tdd": {"type": "string"},
                        "research": {"type": "string"},
                        "data_model": {"type": "string"},
                        "quickstart": {"type": "string"},
                        "contracts": {"type": "string"},
                        "checklists": {"type": "string"},
                    },
                },
                "expected_etag": {"type": "string"},
            },
            "required": ["spec_id", "title", "sections"],
        },
    },
    {
        "name": "spec_issue_get",
        "description": "Get a spec issue by issue number.",
        "inputSchema": {
            "type": "object",
            "properties": {"issue_number": {"type": "integer", "minimum": 1}},
            "required": ["issue_number"],
        },
    },
    {
        "name": "spec_issue_close",
        "description": "Close a spec issue (soft delete).",
        "inputSchema": {
            "type": "object",
            "properties": {"issue_number": {"type": "integer", "minimum": 1}},
            "required": ["issue_number"],
        },
    },
    {
        "name": "spec_issue_artifact_upsert",
        "description": "Create or update artifact comment for contracts/checklists.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "issue_number": {"type": "integer", "minimum": 1},
                "kind": {"type": "string", "enum": ["contract", "checklist"]},
                "artifact_name": {"type": "string"},
                "content": {"type": "string"},
                "expected_etag": {"type": "string"},
            },
            "required": ["issue_number", "kind", "artifact_name", "content"],
        },
    },
    {
        "name": "spec_issue_artifact_list",
        "description": "List artifact comments for contracts/checklists on an issue.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "issue_number": {"type": "integer", "minimum": 1},
                "kind": {"type": "string", "enum": ["contract", "checklist"]},
            },
            "required": ["issue_number"],
        },
    },
    {
        "name": "spec_issue_artifact_delete",
        "description": "Delete artifact comment for contracts/checklists from an issue.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "issue_number": {"type": "integer", "minimum": 1},
                "kind": {"type": "string", "enum": ["contract", "checklist"]},
                "artifact_name": {"type": "string"},
                "expected_etag": {"type": "string"},
            },
            "required": ["issue_number", "kind", "artifact_name"],
        },
    },
    {
        "name": "spec_contract_comment_append",
        "description": "Backward-compatible alias for contract artifact upsert.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "issue_number": {"type": "integer", "minimum": 1},
                "contract_name": {"type": "string"},
                "content": {"type": "string"},
            },
            "required": ["issue_number", "contract_name", "content"],
        },
    },
    {
        "name": "spec_project_sync",
        "description": "Add issue to Project V2 and update Status field.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "issue_number": {"type": "integer", "minimum": 1},
                "project_id": {"type": "string"},
                "phase": {
                    "type": "string",
                    "enum": [
                        "draft",
                        "ready",
                        "planned",
                        "ready-for-dev",
                        "in-progress",
                        "done",
                        "blocked",
                    ],
                },
            },
            "required": ["issue_number", "project_id", "phase"],
        },
    },
]


PHASE_TO_STATUS = {
    "draft": "Draft",
    "ready": "Ready",
    "planned": "Planned",
    "ready-for-dev": "Ready for Dev",
    "in-progress": "In Progress",
    "done": "Done",
    "blocked": "Blocked",
}

ARTIFACT_MARKER_PREFIX = "<!-- GWT_SPEC_ARTIFACT:"
ARTIFACT_MARKER_SUFFIX = " -->"
VALID_ARTIFACT_KINDS = {"contract", "checklist"}

SECTION_ALIASES = {
    "spec": ["spec"],
    "plan": ["plan"],
    "tasks": ["tasks"],
    "tdd": ["tdd"],
    "research": ["research"],
    "data_model": ["data_model", "dataModel"],
    "quickstart": ["quickstart"],
    "contracts": ["contracts"],
    "checklists": ["checklists"],
}


@dataclass
class GhResult:
    code: int
    stdout: str
    stderr: str


def run_gh(*args: str) -> GhResult:
    proc = subprocess.run(
        ["gh", *args],
        text=True,
        capture_output=True,
        check=False,
    )
    return GhResult(code=proc.returncode, stdout=proc.stdout, stderr=proc.stderr)


def require(condition: bool, message: str):
    if not condition:
        raise ValueError(message)


def non_empty(text: Optional[str]) -> str:
    value = (text or "").strip()
    if not value:
        return "_TODO_"
    return value


def parse_issue_number_from_url(url: str) -> int:
    for part in url.strip().split("/"):
        if part.isdigit():
            return int(part)
    raise ValueError(f"Failed to parse issue number from output: {url}")


def build_etag(updated_at: str, body: str) -> str:
    return f"{updated_at.strip()}:{len(body)}"


def normalize_sections(raw: Any) -> Dict[str, str]:
    result: Dict[str, str] = {}
    if not isinstance(raw, dict):
        return result
    for canonical, aliases in SECTION_ALIASES.items():
        for key in aliases:
            if key in raw:
                value = raw.get(key)
                if value is None:
                    result[canonical] = ""
                elif isinstance(value, str):
                    result[canonical] = value
                else:
                    result[canonical] = str(value)
                break
    return result


def merge_sections(base: Dict[str, str], patch: Dict[str, str]) -> Dict[str, str]:
    merged = dict(base)
    for key, value in patch.items():
        merged[key] = value
    return merged


def render_body(spec_id: str, sections: Dict[str, Any]) -> str:
    return "\n".join(
        [
            f"<!-- GWT_SPEC_ID:{spec_id} -->",
            "",
            "## Spec",
            "",
            non_empty(str(sections.get("spec", ""))),
            "",
            "## Plan",
            "",
            non_empty(str(sections.get("plan", ""))),
            "",
            "## Tasks",
            "",
            non_empty(str(sections.get("tasks", ""))),
            "",
            "## TDD",
            "",
            non_empty(str(sections.get("tdd", ""))),
            "",
            "## Research",
            "",
            non_empty(str(sections.get("research", ""))),
            "",
            "## Data Model",
            "",
            non_empty(str(sections.get("data_model", ""))),
            "",
            "## Quickstart",
            "",
            non_empty(str(sections.get("quickstart", ""))),
            "",
            "## Contracts",
            "",
            (
                str(sections.get("contracts", "")).strip()
                or "Artifact files under `contracts/` are managed in issue comments with `contract:<name>` entries."
            ),
            "",
            "## Checklists",
            "",
            (
                str(sections.get("checklists", "")).strip()
                or "Artifact files under `checklists/` are managed in issue comments with `checklist:<name>` entries."
            ),
            "",
            "## Acceptance Checklist",
            "",
            "- [ ] Add acceptance checklist",
        ]
    )


def split_sections(body: str) -> Dict[str, str]:
    out: Dict[str, str] = {}
    current: Optional[str] = None
    lines: list[str] = []

    def flush():
        nonlocal current, lines
        if current is not None:
            out[current] = "\n".join(lines).strip()
        current = None
        lines = []

    for raw in body.splitlines():
        line = raw.rstrip()
        if line.startswith("## "):
            flush()
            current = line[3:].strip()
            continue
        if current is not None:
            lines.append(line)
    flush()
    return out


def parse_issue_detail(obj: Dict[str, Any]) -> Dict[str, Any]:
    labels = []
    for it in obj.get("labels", []) or []:
        name = (it or {}).get("name")
        if isinstance(name, str) and name:
            labels.append(name)
    spec_id = next((l for l in labels if l.startswith("SPEC-") and len(l) == 13), None)
    body = obj.get("body") or ""
    updated_at = obj.get("updatedAt") or ""
    sections = split_sections(body)
    return {
        "number": obj.get("number"),
        "title": obj.get("title") or "",
        "url": obj.get("url") or "",
        "updatedAt": updated_at,
        "labels": labels,
        "specId": spec_id,
        "body": body,
        "etag": build_etag(updated_at, body),
        "sections": {
            "spec": sections.get("Spec", ""),
            "plan": sections.get("Plan", ""),
            "tasks": sections.get("Tasks", ""),
            "tdd": sections.get("TDD", ""),
            "research": sections.get("Research", ""),
            "dataModel": sections.get("Data Model", ""),
            "quickstart": sections.get("Quickstart", ""),
            "contracts": sections.get("Contracts", ""),
            "checklists": sections.get("Checklists", sections.get("Checklist", "")),
        },
    }


def find_issue_by_spec_id(spec_id: str) -> Optional[int]:
    listed = run_gh(
        "issue",
        "list",
        "--state",
        "all",
        "--label",
        spec_id,
        "--json",
        "number",
        "--limit",
        "1",
    )
    if listed.code != 0:
        raise RuntimeError(f"gh issue list failed: {listed.stderr.strip()}")
    arr = json.loads(listed.stdout or "[]")
    if not arr:
        return None
    number = arr[0].get("number")
    return int(number) if isinstance(number, int) else None


def get_issue(issue_number: int) -> Dict[str, Any]:
    viewed = run_gh(
        "issue",
        "view",
        str(issue_number),
        "--json",
        "number,title,body,updatedAt,labels,url,id",
    )
    if viewed.code != 0:
        raise RuntimeError(f"gh issue view failed: {viewed.stderr.strip()}")
    obj = json.loads(viewed.stdout or "{}")
    return parse_issue_detail(obj)


def upsert_issue(args: Dict[str, Any]) -> Dict[str, Any]:
    spec_id = str(args.get("spec_id", "")).strip()
    title = str(args.get("title", "")).strip()
    sections = args.get("sections", {}) or {}
    expected_etag = str(args.get("expected_etag", "")).strip()
    require(spec_id, "spec_id is required")
    require(title, "title is required")
    require(isinstance(sections, dict), "sections must be an object")

    incoming_sections = normalize_sections(sections)
    issue_number = find_issue_by_spec_id(spec_id)

    if issue_number is not None:
        current = get_issue(issue_number)
        if expected_etag and expected_etag != current.get("etag"):
            raise RuntimeError("etag mismatch")
        current_sections = normalize_sections(current.get("sections", {}) or {})
        merged_sections = merge_sections(current_sections, incoming_sections)
        body = render_body(spec_id, merged_sections)
        edited = run_gh(
            "issue",
            "edit",
            str(issue_number),
            "--title",
            title,
            "--body",
            body,
            "--add-label",
            "gwt-spec",
            "--add-label",
            spec_id,
        )
        if edited.code != 0:
            raise RuntimeError(f"gh issue edit failed: {edited.stderr.strip()}")
        return get_issue(issue_number)

    body = render_body(spec_id, incoming_sections)
    created = run_gh(
        "issue",
        "create",
        "--title",
        title,
        "--body",
        body,
        "--label",
        "gwt-spec",
        "--label",
        spec_id,
    )
    if created.code != 0:
        raise RuntimeError(f"gh issue create failed: {created.stderr.strip()}")
    issue_number = parse_issue_number_from_url(created.stdout.strip())
    return get_issue(issue_number)


def close_issue(args: Dict[str, Any]) -> Dict[str, Any]:
    issue_number = int(args.get("issue_number", 0))
    require(issue_number > 0, "issue_number is required")
    closed = run_gh("issue", "close", str(issue_number))
    if closed.code != 0:
        raise RuntimeError(f"gh issue close failed: {closed.stderr.strip()}")
    return {"status": "closed", "issueNumber": issue_number}


def normalize_artifact_kind(value: str) -> str:
    kind = value.strip().lower()
    if kind not in VALID_ARTIFACT_KINDS:
        raise RuntimeError(f"artifact kind is invalid: {value}")
    return kind


def parse_artifact_comment(body: str) -> Optional[Dict[str, str]]:
    text = body or ""
    lines = text.splitlines()
    first_non_empty = next((line for line in lines if line.strip()), "")
    if not first_non_empty:
        return None

    marker = first_non_empty.strip()
    if marker.startswith(ARTIFACT_MARKER_PREFIX) and marker.endswith(
        ARTIFACT_MARKER_SUFFIX
    ):
        payload = marker[len(ARTIFACT_MARKER_PREFIX) : -len(ARTIFACT_MARKER_SUFFIX)].strip()
        if ":" not in payload:
            return None
        kind, artifact_name = payload.split(":", 1)
        try:
            kind = normalize_artifact_kind(kind)
        except RuntimeError:
            return None
        remaining = text.split(first_non_empty, 1)[1].lstrip("\r\n")
        maybe_prefix = f"{kind}:{artifact_name.strip()}"
        if remaining.splitlines() and remaining.splitlines()[0].strip() == maybe_prefix:
            remaining = remaining.split(remaining.splitlines()[0], 1)[1].lstrip("\r\n")
        return {
            "kind": kind,
            "artifactName": artifact_name.strip(),
            "content": remaining.strip(),
        }

    if ":" not in marker:
        return None
    kind, artifact_name = marker.split(":", 1)
    try:
        kind = normalize_artifact_kind(kind)
    except RuntimeError:
        return None
    remaining = text.split(first_non_empty, 1)[1].lstrip("\r\n")
    return {
        "kind": kind,
        "artifactName": artifact_name.strip(),
        "content": remaining.strip(),
    }


def render_artifact_comment(kind: str, artifact_name: str, content: str) -> str:
    k = normalize_artifact_kind(kind)
    name = artifact_name.strip()
    payload = content.strip()
    return (
        f"{ARTIFACT_MARKER_PREFIX}{k}:{name}{ARTIFACT_MARKER_SUFFIX}\n"
        f"{k}:{name}\n\n"
        f"{payload}"
    )


def get_issue_node_and_comments(issue_number: int) -> tuple[str, list[Dict[str, Any]]]:
    viewed = run_gh("issue", "view", str(issue_number), "--json", "id,comments")
    if viewed.code != 0:
        raise RuntimeError(f"gh issue view failed: {viewed.stderr.strip()}")
    obj = json.loads(viewed.stdout or "{}")
    issue_node_id = obj.get("id")
    if not isinstance(issue_node_id, str) or not issue_node_id:
        raise RuntimeError("issue node id not found")
    comments = obj.get("comments", []) or []
    if not isinstance(comments, list):
        comments = []
    return issue_node_id, comments


def to_artifact_entry(issue_number: int, comment: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    comment_id = comment.get("id")
    if not isinstance(comment_id, str) or not comment_id:
        return None
    body = str(comment.get("body") or "")
    parsed = parse_artifact_comment(body)
    if not parsed:
        return None
    updated_at = (
        str(comment.get("updatedAt") or "")
        or str(comment.get("lastEditedAt") or "")
        or str(comment.get("createdAt") or "")
    )
    content = parsed["content"]
    return {
        "commentId": comment_id,
        "issueNumber": issue_number,
        "kind": parsed["kind"],
        "artifactName": parsed["artifactName"],
        "content": content,
        "updatedAt": updated_at,
        "etag": build_etag(updated_at, content),
        "url": comment.get("url"),
    }


def list_artifacts(args: Dict[str, Any]) -> Dict[str, Any]:
    issue_number = int(args.get("issue_number", 0))
    require(issue_number > 0, "issue_number is required")
    kind_filter = str(args.get("kind", "")).strip().lower()
    if kind_filter:
        kind_filter = normalize_artifact_kind(kind_filter)
    _, comments = get_issue_node_and_comments(issue_number)
    items = []
    for raw in comments:
        if not isinstance(raw, dict):
            continue
        parsed = to_artifact_entry(issue_number, raw)
        if not parsed:
            continue
        if kind_filter and parsed.get("kind") != kind_filter:
            continue
        items.append(parsed)
    return {"items": items}


def add_artifact_comment(issue_node_id: str, body: str) -> Dict[str, Any]:
    mutation = (
        "mutation($subject:ID!, $body:String!){"
        " addComment(input:{subjectId:$subject, body:$body}) {"
        " commentEdge { node { id body updatedAt url } }"
        " }"
        "}"
    )
    data = graphql(
        "-f",
        f"query={mutation}",
        "-F",
        f"subject={issue_node_id}",
        "-F",
        f"body={body}",
    )
    node = (
        data.get("data", {})
        .get("addComment", {})
        .get("commentEdge", {})
        .get("node", {})
    )
    if not isinstance(node, dict) or not node:
        raise RuntimeError("added artifact comment payload missing")
    return node


def update_artifact_comment(comment_id: str, body: str) -> Dict[str, Any]:
    mutation = (
        "mutation($id:ID!, $body:String!){"
        " updateIssueComment(input:{id:$id, body:$body}) {"
        " issueComment { id body updatedAt url }"
        " }"
        "}"
    )
    data = graphql(
        "-f",
        f"query={mutation}",
        "-F",
        f"id={comment_id}",
        "-F",
        f"body={body}",
    )
    node = data.get("data", {}).get("updateIssueComment", {}).get("issueComment", {})
    if not isinstance(node, dict) or not node:
        raise RuntimeError("updated artifact comment payload missing")
    return node


def delete_artifact_comment(comment_id: str) -> None:
    mutation = (
        "mutation($id:ID!){"
        " deleteIssueComment(input:{id:$id}) { clientMutationId }"
        "}"
    )
    graphql("-f", f"query={mutation}", "-F", f"id={comment_id}")


def upsert_artifact(args: Dict[str, Any]) -> Dict[str, Any]:
    issue_number = int(args.get("issue_number", 0))
    kind = normalize_artifact_kind(str(args.get("kind", "")))
    artifact_name = str(args.get("artifact_name", "")).strip()
    content = str(args.get("content", "")).strip()
    expected_etag = str(args.get("expected_etag", "")).strip()
    require(issue_number > 0, "issue_number is required")
    require(artifact_name, "artifact_name is required")
    require(content, "content is required")

    issue_node_id, comments = get_issue_node_and_comments(issue_number)
    existing = None
    for raw in comments:
        if not isinstance(raw, dict):
            continue
        parsed = to_artifact_entry(issue_number, raw)
        if not parsed:
            continue
        if parsed.get("kind") == kind and parsed.get("artifactName") == artifact_name:
            existing = parsed
            break

    body = render_artifact_comment(kind, artifact_name, content)
    if existing is not None:
        if expected_etag and expected_etag != str(existing.get("etag") or ""):
            raise RuntimeError("etag mismatch")
        updated_raw = update_artifact_comment(str(existing["commentId"]), body)
        updated = to_artifact_entry(issue_number, updated_raw)
        if not updated:
            raise RuntimeError("failed to parse updated artifact comment")
        return updated

    created_raw = add_artifact_comment(issue_node_id, body)
    created = to_artifact_entry(issue_number, created_raw)
    if not created:
        raise RuntimeError("failed to parse created artifact comment")
    return created


def delete_artifact(args: Dict[str, Any]) -> Dict[str, Any]:
    issue_number = int(args.get("issue_number", 0))
    kind = normalize_artifact_kind(str(args.get("kind", "")))
    artifact_name = str(args.get("artifact_name", "")).strip()
    expected_etag = str(args.get("expected_etag", "")).strip()
    require(issue_number > 0, "issue_number is required")
    require(artifact_name, "artifact_name is required")

    _, comments = get_issue_node_and_comments(issue_number)
    target = None
    for raw in comments:
        if not isinstance(raw, dict):
            continue
        parsed = to_artifact_entry(issue_number, raw)
        if not parsed:
            continue
        if parsed.get("kind") == kind and parsed.get("artifactName") == artifact_name:
            target = parsed
            break

    if target is None:
        return {"deleted": False}
    if expected_etag and expected_etag != str(target.get("etag") or ""):
        raise RuntimeError("etag mismatch")
    delete_artifact_comment(str(target["commentId"]))
    return {"deleted": True}


def append_contract(args: Dict[str, Any]) -> Dict[str, Any]:
    # Backward-compatible alias.
    return upsert_artifact(
        {
            "issue_number": args.get("issue_number"),
            "kind": "contract",
            "artifact_name": args.get("contract_name"),
            "content": args.get("content"),
        }
    )


def graphql(*args: str) -> Dict[str, Any]:
    out = run_gh("api", "graphql", *args)
    if out.code != 0:
        raise RuntimeError(out.stderr.strip() or "graphql failed")
    return json.loads(out.stdout or "{}")


def ensure_project_item(project_id: str, issue_node_id: str) -> str:
    add_query = (
        "mutation($project:ID!, $content:ID!){"
        " addProjectV2ItemById(input:{projectId:$project, contentId:$content}) { item { id } }"
        "}"
    )
    try:
        data = graphql(
            "-f",
            f"query={add_query}",
            "-F",
            f"project={project_id}",
            "-F",
            f"content={issue_node_id}",
        )
        item_id = (
            data.get("data", {})
            .get("addProjectV2ItemById", {})
            .get("item", {})
            .get("id")
        )
        if isinstance(item_id, str) and item_id:
            return item_id
    except Exception:
        pass

    list_query = (
        "query($project:ID!){"
        " node(id:$project){ ... on ProjectV2 { items(first:100){ nodes { id content { ... on Issue { id } } } } } }"
        "}"
    )
    data = graphql("-f", f"query={list_query}", "-F", f"project={project_id}")
    nodes = (
        data.get("data", {})
        .get("node", {})
        .get("items", {})
        .get("nodes", [])
    )
    for node in nodes:
        content_id = ((node or {}).get("content") or {}).get("id")
        if content_id == issue_node_id:
            item_id = (node or {}).get("id")
            if isinstance(item_id, str) and item_id:
                return item_id
    raise RuntimeError("failed to resolve project item id")


def update_project_status(project_id: str, item_id: str, status_name: str) -> bool:
    field_query = (
        "query($project:ID!){"
        " node(id:$project){ ... on ProjectV2 { fields(first:100){ nodes {"
        " ... on ProjectV2SingleSelectField { id name options { id name } }"
        " } } } }"
        "}"
    )
    data = graphql("-f", f"query={field_query}", "-F", f"project={project_id}")
    nodes = (
        data.get("data", {})
        .get("node", {})
        .get("fields", {})
        .get("nodes", [])
    )
    field_id = None
    option_id = None
    for node in nodes:
        if (node or {}).get("name") != "Status":
            continue
        field_id = (node or {}).get("id")
        for opt in (node or {}).get("options", []) or []:
            if (opt or {}).get("name") == status_name:
                option_id = (opt or {}).get("id")
                break
        break
    if not field_id:
        raise RuntimeError("Status field not found")
    if not option_id:
        raise RuntimeError(f"Status option not found: {status_name}")

    mutation = (
        "mutation($project:ID!, $item:ID!, $field:ID!, $option:String!){"
        " updateProjectV2ItemFieldValue(input:{"
        " projectId:$project, itemId:$item, fieldId:$field, value:{ singleSelectOptionId:$option }"
        " }) { projectV2Item { id } }"
        "}"
    )
    graphql(
        "-f",
        f"query={mutation}",
        "-F",
        f"project={project_id}",
        "-F",
        f"item={item_id}",
        "-F",
        f"field={field_id}",
        "-F",
        f"option={option_id}",
    )
    return True


def sync_project(args: Dict[str, Any]) -> Dict[str, Any]:
    issue_number = int(args.get("issue_number", 0))
    project_id = str(args.get("project_id", "")).strip()
    phase = str(args.get("phase", "")).strip().lower()
    require(issue_number > 0, "issue_number is required")
    require(project_id, "project_id is required")
    require(phase in PHASE_TO_STATUS, "phase is invalid")

    issue = run_gh("issue", "view", str(issue_number), "--json", "id")
    if issue.code != 0:
        raise RuntimeError(f"gh issue view failed: {issue.stderr.strip()}")
    issue_node_id = (json.loads(issue.stdout or "{}")).get("id")
    if not isinstance(issue_node_id, str) or not issue_node_id:
        raise RuntimeError("issue node id not found")

    item_id = ensure_project_item(project_id, issue_node_id)
    status_name = PHASE_TO_STATUS[phase]
    status_applied = update_project_status(project_id, item_id, status_name)
    return {
        "projectItemId": item_id,
        "statusApplied": status_applied,
        "warning": None,
    }


def handle_tool_call(name: str, args: Dict[str, Any]) -> Dict[str, Any]:
    if name == "spec_issue_upsert":
        return upsert_issue(args)
    if name == "spec_issue_get":
        issue_number = int(args.get("issue_number", 0))
        require(issue_number > 0, "issue_number is required")
        return get_issue(issue_number)
    if name == "spec_issue_close":
        return close_issue(args)
    if name == "spec_issue_artifact_upsert":
        return upsert_artifact(args)
    if name == "spec_issue_artifact_list":
        return list_artifacts(args)
    if name == "spec_issue_artifact_delete":
        return delete_artifact(args)
    if name == "spec_contract_comment_append":
        return append_contract(args)
    if name == "spec_project_sync":
        return sync_project(args)
    raise ValueError(f"Unknown tool: {name}")


def write_message(payload: Dict[str, Any]):
    raw = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    sys.stdout.write(f"Content-Length: {len(raw)}\r\n\r\n")
    sys.stdout.flush()
    sys.stdout.buffer.write(raw)
    sys.stdout.buffer.flush()


def read_message() -> Optional[Dict[str, Any]]:
    headers: Dict[str, str] = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b"\r\n", b"\n"):
            break
        key, _, value = line.decode("utf-8").partition(":")
        headers[key.strip().lower()] = value.strip()
    length = int(headers.get("content-length", "0"))
    if length <= 0:
        return None
    body = sys.stdin.buffer.read(length)
    if not body:
        return None
    return json.loads(body.decode("utf-8"))


def success_response(req_id: Any, result: Dict[str, Any]) -> Dict[str, Any]:
    return {"jsonrpc": "2.0", "id": req_id, "result": result}


def error_response(req_id: Any, message: str, code: int = -32000) -> Dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": req_id,
        "error": {"code": code, "message": message},
    }


def main():
    while True:
        req = read_message()
        if req is None:
            return
        method = req.get("method")
        req_id = req.get("id")
        params = req.get("params") or {}

        try:
            if method == "initialize":
                write_message(
                    success_response(
                        req_id,
                        {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {"tools": {}},
                            "serverInfo": {
                                "name": "gwt-issue-spec-mcp",
                                "version": "0.1.0",
                            },
                        },
                    )
                )
                continue

            if method == "initialized":
                continue

            if method == "tools/list":
                write_message(success_response(req_id, {"tools": TOOLS}))
                continue

            if method == "tools/call":
                name = str(params.get("name", ""))
                arguments = params.get("arguments") or {}
                if not isinstance(arguments, dict):
                    raise ValueError("arguments must be an object")
                result = handle_tool_call(name, arguments)
                write_message(
                    success_response(
                        req_id,
                        {
                            "content": [
                                {
                                    "type": "text",
                                    "text": json.dumps(result, ensure_ascii=False),
                                }
                            ],
                            "structuredContent": result,
                        },
                    )
                )
                continue

            if method == "ping":
                write_message(success_response(req_id, {}))
                continue

            if req_id is not None:
                write_message(error_response(req_id, f"Unsupported method: {method}", -32601))
        except Exception as exc:
            if req_id is None:
                continue
            write_message(error_response(req_id, str(exc)))


if __name__ == "__main__":
    main()
