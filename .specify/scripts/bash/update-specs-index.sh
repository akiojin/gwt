#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(CDPATH="" cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

REPO_ROOT="$(get_repo_root)"
if [ -z "$REPO_ROOT" ]; then
    echo "ERROR: リポジトリルートを特定できません" >&2
    exit 1
fi

SPECS_DIR="$REPO_ROOT/specs"
ARCHIVE_DIR="$SPECS_DIR/archive"
OUTPUT="$SPECS_DIR/specs.md"
TODAY="$(date +%Y-%m-%d)"

if command -v python3 >/dev/null 2>&1; then
    PY_BIN=python3
elif command -v python >/dev/null 2>&1; then
    PY_BIN=python
else
    echo "ERROR: python が見つかりません" >&2
    exit 1
fi

"$PY_BIN" - <<'PY' "$SPECS_DIR" "$ARCHIVE_DIR" "$OUTPUT" "$TODAY"
import re
import sys
from pathlib import Path

specs_dir = Path(sys.argv[1])
archive_dir = Path(sys.argv[2])
output = Path(sys.argv[3])
today = sys.argv[4]

spec_id_re = re.compile(r"^SPEC-[a-f0-9]{8}$", re.I)

def parse_spec(path: Path):
    text = path.read_text(encoding="utf-8")
    title = ""
    created = "-"
    category = ""

    for line in text.splitlines():
        if line.startswith("# "):
            title = line[2:].strip()
            break

    for line in text.splitlines():
        m = re.match(r"^\*\*作成日\*\*:\s*(.+)$", line)
        if m:
            created = m.group(1).strip()
            break

    for line in text.splitlines():
        m = re.match(r"^\*\*カテゴリ\*\*:\s*(.+)$", line)
        if m:
            category = m.group(1).strip()
            break

    return title or path.parent.name, created, category

def collect_specs(base: Path, prefix: str = ""):
    rows = []
    if not base.exists():
        return rows
    for child in sorted(base.iterdir()):
        if not child.is_dir():
            continue
        if not spec_id_re.match(child.name):
            continue
        spec_file = child / "spec.md"
        if not spec_file.exists():
            continue
        title, created, category = parse_spec(spec_file)
        link = f"{prefix}{child.name}/spec.md"
        rows.append({
            "id": child.name,
            "title": title,
            "created": created,
            "category": category,
            "link": link,
        })
    return rows

current_rows = collect_specs(specs_dir)
archive_rows = collect_specs(archive_dir, prefix="archive/")

# 分類
current_gui = []
current_porting = []
for row in current_rows:
    category = row["category"].lower()
    if "porting" in category or "移植" in category:
        current_porting.append(row)
    else:
        current_gui.append(row)

# 作成日で降順ソート（不明は末尾）

def sort_key(row):
    date = row["created"]
    if re.match(r"^\d{4}-\d{2}-\d{2}$", date):
        return date
    return "0000-00-00"

for rows in (current_gui, current_porting, archive_rows):
    rows.sort(key=sort_key, reverse=True)

# 既存ファイルから運用ルールを抽出（あれば）
ops_rules = None
if output.exists():
    content = output.read_text(encoding="utf-8")
    m = re.search(r"## 運用ルール\n\n(.*?)\n\n## ", content, re.S)
    if m:
        ops_rules = m.group(1).strip()

if not ops_rules:
    ops_rules = "\n".join([
        "- `カテゴリ: GUI` は、現行のTauri GUI実装で有効な要件（binding）です。",
        "- `カテゴリ: Porting` は、TUI/WebUI由来の移植待ち（non-binding）です。未実装でも不具合ではありません。",
        "- Porting を実装対象にする場合は、次のどちらかを実施します:",
        "1. 既存 spec の内容を GUI 前提に更新し、`カテゴリ` を `GUI` に変更する",
        "2. 新しい GUI spec を作成し、元の Porting spec を `**依存仕様**:` で参照する",
    ])

lines = []
lines.append("# 仕様一覧")
lines.append("")
lines.append(f"**最終更新**: {today}")
lines.append("")
lines.append("- 現行の仕様・要件: `specs/SPEC-XXXXXXXX/spec.md`")
lines.append("- 以前までのTUIの仕様・要件（archive）: `specs/archive/SPEC-XXXXXXXX/spec.md`")
lines.append("")
lines.append("## 運用ルール")
lines.append("")
lines.append(ops_rules)
lines.append("")

lines.append("## 現行仕様（GUI）")
lines.append("")
lines.append("| SPEC ID | タイトル | 作成日 |")
lines.append("| --- | --- | --- |")
for row in current_gui:
    lines.append(f"| [{row['id']}]({row['link']}) | {row['title']} | {row['created']} |")
lines.append("")

lines.append("## 移植待ち（Porting）")
lines.append("")
lines.append("| SPEC ID | タイトル | 作成日 |")
lines.append("| --- | --- | --- |")
for row in current_porting:
    lines.append(f"| [{row['id']}]({row['link']}) | {row['title']} | {row['created']} |")
lines.append("")

lines.append("## 過去要件（archive）")
lines.append("")
lines.append("| SPEC ID | タイトル | 作成日 |")
lines.append("| --- | --- | --- |")
for row in archive_rows:
    lines.append(f"| [{row['id']}]({row['link']}) | {row['title']} | {row['created']} |")
lines.append("")

output.write_text("\n".join(lines), encoding="utf-8")
PY
