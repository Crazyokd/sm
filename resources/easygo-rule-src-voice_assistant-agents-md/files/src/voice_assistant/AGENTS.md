# Repository Guidelines

## Project Structure & Module Organization

This repository is the EasyGo voice assistant backend. `app/` contains the Python backend. Key areas are `app/api/` for FastAPI routes, `app/services/` for ASR, task routing, TTS, task gateway, and validation logic. Tests live in `tests/`. Runtime config is in `config/`, and developer, testing, and local-model deployment notes are in `docs/`.

The current EasyGo voice assistant frontend is not maintained in this repository. Frontend UI, recorder entry points, panels, and page state live in the EasyGo frontend repository (`standard-fe/easygo`, checked out at `src/easygo-fe` in the EasyGo workspace), especially `src/components/voice-assistant/`, `src/modules/voice-assistant/`, and `src/pages/voice-assistant/`.

## Build, Test, and Development Commands

- The backend runs only inside full simulation or on the robot (`/sros/voice-assistant/current/bin/voice-assistant`, port `18091`); it is not started directly from this checkout.
- `PYTHONPATH=/opt/easygo-voice-assistant/python_deps python3 -m pytest tests -q`: run backend tests inside the `easygo-dev-main` container.
- `bash scripts/deploy/build-deb.sh`: build the `easygo-voice-assistant` deb.

## Coding Style & Naming Conventions

Use 4-space indentation in Python. Follow existing naming: snake_case for Python functions and modules, and descriptive test names starting with `test_`. Keep service boundaries explicit: parsing, network calls, validation, and narration should stay in separate modules.

## Testing Guidelines

Backend tests use `pytest`. Add or update tests for every behavior change, especially around ordinary-mode rule responses, inventory validation, and script behavior. Prefer focused unit tests over broad end-to-end coverage. Mock external services such as DashScope, Vosk, and Edge TTS in tests.

## Commit & Pull Request Guidelines

Recent history uses short conventional prefixes such as `feat:`, `fix:`, `docs:`, and `chore:`. Keep commit subjects imperative and specific, for example: `fix: reject unsupported ordinary-mode input`. PRs should state user-visible changes, config changes, and test evidence.

## Security & Configuration Tips

Do not hardcode API keys. Use `LLM_API_KEY` in `.env.runtime` for cloud ASR and TTS; Ultra uses `ULTRA_LLM_API_KEY`. Ordinary mode uses deterministic rules; Ultra owns model-based planning. All other operator voice settings come from easygo-app function parameters (`voice.*`). Keep local models and runtime artifacts out of git. Verify the relevant API key and backend port before debugging application logic.


<claude-mem-context>
# Memory Context

# [voice_assistant/voice_assistant] recent context, 2026-05-18 3:55pm GMT+8

No previous sessions found.
</claude-mem-context>
