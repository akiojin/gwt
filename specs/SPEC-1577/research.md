### Responses API tools 形式

**OpenAI Responses API (tools):**
```json
{
  "type": "function",
  "name": "tool_name",
  "description": "Tool description",
  "parameters": {
    "type": "object",
    "properties": { ... },
    "required": [ ... ]
  }
}
```

**Anthropic Claude (tools API):**
```json
{
  "name": "tool_name",
  "description": "Tool description",
  "input_schema": {
    "type": "object",
    "properties": { ... },
    "required": [ ... ]
  }
}
```

設計方針: Responses API (OpenAI互換) の tools 形式を主とし、必要に応じて Anthropic 形式への変換層を用意する。

---
