#!/usr/bin/env python3
"""Qwen3-ASR helper for gwt voice input commands.

This helper is executed by Rust backend commands and returns JSON on stdout.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from typing import Optional


def emit(payload: dict) -> None:
    sys.stdout.write(json.dumps(payload, ensure_ascii=False))
    sys.stdout.flush()


def map_language(value: str) -> Optional[str]:
    normalized = (value or "").strip().lower()
    if normalized == "ja":
        return "Japanese"
    if normalized == "en":
        return "English"
    return None


def probe_runtime() -> dict:
    missing = []
    if importlib.util.find_spec("qwen_asr") is None:
        missing.append("qwen_asr")
    if importlib.util.find_spec("torch") is None:
        missing.append("torch")

    if missing:
        return {
            "ok": False,
            "error": f"Missing Python package(s): {', '.join(missing)}",
        }

    return {
        "ok": True,
        "pythonVersion": sys.version.split()[0],
    }


def load_model(model_id: str):
    from qwen_asr import Qwen3ASRModel  # type: ignore

    return Qwen3ASRModel.from_pretrained(
        model_id,
        dtype="auto",
        device_map="auto",
    )


def run_prepare(model_id: str) -> dict:
    _ = load_model(model_id)
    return {
        "ok": True,
        "modelId": model_id,
    }


def extract_transcript(result: object) -> str:
    if result is None:
        return ""

    if isinstance(result, str):
        return result.strip()

    if isinstance(result, dict):
        value = result.get("text") or result.get("transcript") or ""
        return str(value).strip()

    if isinstance(result, list):
        for item in result:
            text = extract_transcript(item)
            if text:
                return text
        return ""

    text_attr = getattr(result, "text", None)
    if text_attr is not None:
        return str(text_attr).strip()

    transcript_attr = getattr(result, "transcript", None)
    if transcript_attr is not None:
        return str(transcript_attr).strip()

    return ""


def run_transcribe(model_id: str, audio_path: str, language: str) -> dict:
    model = load_model(model_id)
    language_name = map_language(language)
    result = model.transcribe(audio_path, language=language_name)

    transcript = extract_transcript(result)

    return {
        "ok": True,
        "modelId": model_id,
        "transcript": transcript,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="gwt Qwen3-ASR helper")
    parser.add_argument(
        "--action",
        required=True,
        choices=["probe", "prepare", "transcribe"],
    )
    parser.add_argument("--model-id", default="")
    parser.add_argument("--audio-path", default="")
    parser.add_argument("--language", default="auto")
    return parser.parse_args()


def main() -> int:
    args = parse_args()

    try:
        if args.action == "probe":
            emit(probe_runtime())
            return 0

        if args.action == "prepare":
            if not args.model_id:
                emit({"ok": False, "error": "--model-id is required for prepare"})
                return 2
            emit(run_prepare(args.model_id))
            return 0

        if args.action == "transcribe":
            if not args.model_id:
                emit({"ok": False, "error": "--model-id is required for transcribe"})
                return 2
            if not args.audio_path:
                emit({"ok": False, "error": "--audio-path is required for transcribe"})
                return 2
            emit(run_transcribe(args.model_id, args.audio_path, args.language))
            return 0

        emit({"ok": False, "error": f"Unsupported action: {args.action}"})
        return 2

    except Exception as exc:
        emit(
            {
                "ok": False,
                "error": str(exc),
                "exceptionType": type(exc).__name__,
            }
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
