# Voice Evaluation Dataset

This directory contains local assets for offline voice accuracy checks.

## Files

- `script-ja.txt`: Japanese recording script for consistent test utterances.
- `manifest.template.json`: template for your local `manifest.json`.
- `samples/`: place your recorded WAV files here.

## Quick Start

1. Copy template:
```bash
cp tests/voice_eval/manifest.template.json tests/voice_eval/manifest.json
```

1. Record your voice using `tests/voice_eval/script-ja.txt` and place WAV files under:
- `tests/voice_eval/samples/001.wav`
- `tests/voice_eval/samples/002.wav`
- ...

1. Run evaluation:
```bash
scripts/voice-eval.sh
```

1. Run additional popular models:
```bash
scripts/voice-eval.sh --models popular-lite
```

1. Run full-size popular models (large downloads):
```bash
scripts/voice-eval.sh --models popular
```

1. Optional baseline update (after you approve current quality):
```bash
cp tests/voice_eval/latest-report.json tests/voice_eval/baseline.json
```

## WAV Requirements

- PCM WAV
- mono or stereo (stereo will be downmixed)
- recommended sample rate: 16kHz or 48kHz

## Notes

- `manifest.json`, `baseline.json`, `latest-report.json`, and `samples/*.wav` are ignored by git.
- The evaluator computes WER/CER and compares against baseline when available.
- A versioned summary snapshot is tracked at `docs/voice-eval-benchmarks.md`.
