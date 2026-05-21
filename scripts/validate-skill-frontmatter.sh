#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

MANIFEST="$TMP_DIR/frontmatter-manifest.tsv"
FAILURES=0

extract_frontmatter() {
  local input_file="$1"
  local output_file="$2"

  awk '
    BEGIN {
      state = 0
    }
    {
      line = $0
      sub(/\r$/, "", line)

      if (NR == 1) {
        if (line != "---") {
          exit 2
        }
        state = 1
        next
      }

      if (state == 1 && line == "---") {
        state = 2
        exit 0
      }

      if (state == 1) {
        print line
      }
    }
    END {
      if (NR == 0) {
        exit 2
      }
      if (state != 2) {
        exit 3
      }
    }
  ' "$input_file" > "$output_file"
}

SKILL_FILES=()
while IFS= read -r skill_file; do
  SKILL_FILES+=("$skill_file")
done < <(git ls-files '*SKILL.md')

for index in "${!SKILL_FILES[@]}"; do
  relative_path="${SKILL_FILES[$index]}"
  if [ ! -f "$relative_path" ]; then
    echo "$relative_path: tracked SKILL.md is missing from working tree" >&2
    FAILURES=1
    continue
  fi
  frontmatter_file="$TMP_DIR/frontmatter-$index.yaml"

  if extract_frontmatter "$relative_path" "$frontmatter_file"; then
    printf '%s\t%s\n' "$frontmatter_file" "$relative_path" >> "$MANIFEST"
    continue
  fi

  case $? in
    2)
      echo "$relative_path: missing YAML frontmatter block" >&2
      ;;
    3)
      echo "$relative_path: missing closing YAML frontmatter delimiter" >&2
      ;;
    *)
      echo "$relative_path: failed to extract YAML frontmatter" >&2
      ;;
  esac

  FAILURES=1
done

if [ -s "$MANIFEST" ]; then
  if command -v ruby >/dev/null 2>&1; then
    if ! MANIFEST="$MANIFEST" ruby <<'RUBY'
require "yaml"

status = 0
File.readlines(ENV.fetch("MANIFEST"), chomp: true).each do |line|
  yaml_file, source_path = line.split("\t", 2)
  begin
    YAML.safe_load(File.read(yaml_file), permitted_classes: [], aliases: false)
  rescue StandardError => e
    warn "#{source_path}: invalid YAML frontmatter: #{e.message}"
    status = 1
  end
end
exit status
RUBY
    then
      FAILURES=1
    fi
  else
    echo "ruby is required to validate SKILL.md YAML frontmatter." >&2
    FAILURES=1
  fi
fi

if [ "$FAILURES" -ne 0 ]; then
  echo "SKILL.md frontmatter validation failed." >&2
  exit 1
fi

echo "Validated ${#SKILL_FILES[@]} SKILL.md frontmatter block(s)."
