# Plan: SPEC-1784 — SPEC セマンティック検索と検索命名規約

## Summary

project / issue / spec の semantic search API を揃え、欠けている `search-specs` 系 action を定義する。

## Technical Context

- `chroma_index_runner.py` 系の action naming
- `SPEC-1643` が GitHub discovery/search を担当。
- `SPEC-1579` が gwt-spec-search skill workflow を担当。

## Phased Implementation

### Phase 1: API Inventory

1. 既存 files / issues action と output key を棚卸しする。

### Phase 2: SPEC Search

1. index-specs / search-specs と output shape を定義する。

### Phase 3: Integration

1. skill / SPECs tab 連携と backward compatibility を確認する。
