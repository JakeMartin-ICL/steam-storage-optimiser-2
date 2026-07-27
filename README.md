# Steam Storage Optimiser v2

A local-first desktop application for comparing the value of games in a Steam
library by hours played per gigabyte.

This repository is the successor to
[`JakeMartin-ICL/steam-storage-optimiser`](https://github.com/JakeMartin-ICL/steam-storage-optimiser).
The original Python repository is intentionally preserved as a separate,
working legacy project.

Development should begin with the Steam protocol feasibility spike described
in [`docs/v2-spec.md`](docs/v2-spec.md). Do not scaffold the full product until
that spike has established whether QR authentication, library retrieval, and
depot manifest sizing are sufficiently reliable.
