# Imported resource bundle

User-created iteration cleanup skill captured from the local Codex skill directory.

Targets retain their existing scope and exact relative paths. This bundle is staged for review only. No live configuration or ownership is changed by this import. Adoption fingerprints permit an initial ownership transfer only after review, merge, and the ordinary approved apply. Native secrets remain local through `${SM_LOCAL}` leaf markers. Existing hook definitions are preserved; no hook deduplication or trust-state import is performed.

## Project scope correction

The physical truth/contact, MCAP, run UUID and reproducibility requirements belong to simenv. The reviewed correction moves this self-maintained resource to the existing simenv project at `~/workspace/simenv`, projects its complete directory under `.agents/skills/iteration-cleanup`, and removes obsolete user-scope adoption records. Cleanup safeguards are preserved; this change does not execute cleanup.
