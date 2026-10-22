# Imported resource bundle

Local adaptation captured from existing user/project resource locations. Source provenance is unverified for the snapshots in this bundle; this does not claim that third-party content was authored by this user. Known ECC upstream resources are registered separately and regenerated from their upstream provider.

Targets retain their existing scope and exact relative paths. This bundle is staged for review only. No live configuration or ownership is changed by this import. Adoption fingerprints permit an initial ownership transfer only after review, merge, and the ordinary approved apply. Native secrets remain local through `${SM_LOCAL}` leaf markers. Existing hook definitions are preserved; no hook deduplication or trust-state import is performed.

## Additional legacy file coverage

This review adds 4 exact-path local file snapshots, including locally installed or generated commands and client copies where present. These snapshots preserve current bytes and executable flags. Their original authorship or upstream version is not asserted; known unmodified ECC files remain in the upstream selection instead.

## MCP failure isolation

MCP sections are declared in separate bundles for each native configuration file and scope. A missing private MCP value blocks that MCP resource without preventing ordinary rules, scripts, hook definitions, or unrelated clients from rendering on a new machine. Existing non-MCP sections retain their current values.
