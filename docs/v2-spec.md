# Steam Storage Optimiser v2 — Product and Implementation Specification

Status: initial handoff specification

Date: 2026-07-27

Legacy repository: <https://github.com/JakeMartin-ICL/steam-storage-optimiser>

Legacy local path at the time of handoff:
`/Users/jakemartin/Projects/steam-storage-optimiser`

## 1. Context for a new Codex task

Steam Storage Optimiser helps someone decide which games deserve space on
their computer. Its central metric is **lifetime hours played per GB of
installed size**. Despite an early conversational typo, price is not part of
the metric.

The legacy application is a small Python terminal program. It:

1. asks for a Steam Web API key, SteamID64, and Steam installation path;
2. calls `IPlayerService/GetOwnedGames` for names and lifetime playtime;
3. discovers Steam library folders from `libraryfolders.vdf`;
4. reads installed `appmanifest_*.acf` files and their `SizeOnDisk`;
5. obtains estimates for uninstalled games from a crowdsourced AWS database;
6. uploads newly observed or substantially changed installed sizes; and
7. prints a table sorted by hours played per byte, with cumulative size and
   playtime.

The relevant legacy implementation is in
`src/steamStorageOptimiser.py`. The old repository should remain intact: it is
a working historical project with its own Python packaging and release setup.
This v2 repository is intentionally separate.

The desired successor is a Tauri 2 desktop application for Windows, macOS, and
Linux. It should remain local-first, replace the terminal table with a useful
GUI, avoid asking users to find a SteamID manually, and preferably avoid asking
for a Steam Web API key.

The most important new technical hypothesis is:

> A Steam client-protocol login can retrieve the user's owned library and
> playtime locally, while Steam depot manifests can provide uncompressed base
> install sizes for uninstalled games. The existing crowdsourced database
> remains valuable for launcher/bootstrapper games and other cases where Steam
> depot content does not represent the eventual installed footprint.

Do not begin by implementing the entire product. Begin with the feasibility
spike in section 12.

## 2. Decisions already made

These are product decisions, not suggestions:

- The metric is hours played per GB.
- The new application will use Tauri 2.
- Windows, macOS, and Linux are all target platforms.
- v2 lives in this separate repository; do not restructure the legacy repo.
- Steam store system requirements are not a size source. They are too stale,
  inconsistent, and free-form.
- Installed games use the local Steam manifest's actual `SizeOnDisk`.
- Uninstalled games should use both:
  - a depot-manifest estimate; and
  - the crowdsourced installed-size database, where available.
- Users must be able to compare or switch between depot and community sizes.
- Depot download/compressed bytes must never be used as installed size.
  Use manifest uncompressed file sizes.
- Owned DLC that Steam would normally install must be included in the depot
  estimate where possible.
- The existing crowdsourced AWS service should be retained, though its schema
  and API may later be versioned or improved.

## 3. Explicit non-goals for the first usable release

- Game purchase price or price-per-GB.
- Downloading game content.
- Installing, uninstalling, moving, or modifying Steam games.
- Replacing Steam's storage manager.
- Scraping SteamDB.
- Treating Steam store requirements as a fallback.
- Counting workshop content, mods, shader caches, saves, or arbitrary
  post-install launcher downloads as if Steam manifests knew their size.
- Mobile platforms.

## 4. Authentication and library retrieval

### 4.1 Preferred direction: Steam client-protocol QR login

The feasibility spike should first test a local Steam client-protocol session:

1. generate a QR login challenge;
2. display it to the user;
3. let the user approve it in the official Steam Mobile application;
4. receive a Steam client refresh token without handling the password;
5. connect with a distinct logon ID so the normal Steam client is not replaced;
6. retrieve the authenticated account's owned applications and lifetime
   playtime;
7. retrieve licenses/owned depots and PICS product metadata; and
8. retrieve current public-branch depot manifests without content chunks.

Potential implementations to evaluate:

- `steam-cm-protocol` for native Rust QR authentication, library/playtime, and
  PICS metadata:
  <https://docs.rs/steam-cm-protocol/latest/steam_cm_protocol/>
- `steamroom` for native Rust authentication, manifest parsing, and Steam CDN
  access:
  <https://docs.rs/steamroom/latest/steamroom/>
- SteamKit2 as the established fallback:
  <https://github.com/SteamRE/SteamKit>
- DepotDownloader as behavioral reference, especially for depot filtering and
  manifest retrieval:
  <https://github.com/SteamRE/DepotDownloader>

The Rust libraries are currently young `0.x` projects. Inspect their source,
maintenance state, licenses, protocol coverage, and interoperability before
adopting them. If the native path is unreliable, document that finding and
prototype a small self-contained SteamKit2 sidecar rather than forcing an
unstable Rust solution.

### 4.2 Fallback: user API key with automatic SteamID detection

If Steam client-protocol login is rejected or temporarily unavailable, retain a
fallback that:

- detects locally known accounts from Steam's `config/loginusers.vdf`;
- lets the user choose an account if several exist; and
- asks only for the user's Steam Web API key.

This removes the legacy application's manual SteamID lookup.

### 4.3 Deferred fallback: OpenID plus AWS-held API key

An OpenID/backend design is possible but should not be built until the local
Steam protocol spike has been assessed.

A regular Steam Web API key is a caller/application credential, not a token
restricted to the key owner's SteamID. `GetOwnedGames` can query another
SteamID when that account's Game Details are visible to the key owner. A
centrally held key would normally require public Game Details. Valve publishes
a limit of 100,000 Web API calls per key per day, which is ample for the
expected scale.

References:

- <https://partner.steamgames.com/doc/webapi/IPlayerService>
- <https://steamcommunity.com/dev/apiterms>
- <https://steamcommunity.com/dev>

Never embed a shared Steam Web API key in a distributed desktop binary.

## 5. Authentication security requirements

A Steam client refresh token is substantially more sensitive than OpenID,
which returns only a SteamID.

- Never collect or store a Steam password.
- QR approval through Steam Mobile is the preferred login.
- Default to an ephemeral session during the spike.
- If persistent login is later offered, make it an explicit “Remember this
  account” choice and store the token through the operating system credential
  store:
  - Windows Credential Manager;
  - macOS Keychain;
  - Secret Service-compatible storage on Linux.
- Never write refresh tokens, access tokens, cookies, API keys, or depot keys
  to logs, telemetry, frontend state persistence, plaintext JSON, crash
  reports, or test fixtures.
- Keep sensitive Steam state in the Rust/backend process and expose only narrow
  typed Tauri commands to the webview.
- Use a distinct Steam logon ID so the existing Steam client session is not
  displaced.
- Provide logout that disconnects and deletes locally retained credentials.
- Explain in the UI that QR approval authorizes this application as a Steam
  client session.
- Treat frontend injection as capable of invoking allowed Tauri commands.
  Minimize command capabilities and validate every argument in Rust.

## 6. Local Steam discovery

Implement platform-specific discovery behind a common interface.

Expected default roots:

- Windows: Steam registry locations, then common Program Files paths.
- macOS: `~/Library/Application Support/Steam`.
- Linux: common native, distro, and user-data locations such as
  `~/.steam/steam` and `~/.local/share/Steam`.

Use Steam files rather than recursively scanning arbitrary disks:

- `config/loginusers.vdf` for locally known accounts;
- `steamapps/libraryfolders.vdf` for library roots;
- `steamapps/appmanifest_<appid>.acf` for installed state and `SizeOnDisk`.

Support missing/offline external libraries without failing the whole scan.
Filesystem access is read-only. Tauri filesystem/IPC permissions should be
limited to the required Steam locations and application-owned configuration.

## 7. Size domain model

Do not reduce all size data to one unexplained integer. A game may have:

```text
InstalledObservation
  app_id
  bytes
  library_path
  build_id?
  observed_at

DepotEstimate
  app_id
  bytes_uncompressed
  platform
  architecture
  language
  public_build_id
  selected_depot_ids
  included_dlc_app_ids
  manifest_ids
  calculated_at
  warnings[]

CommunityEstimate
  app_id
  representative_bytes
  platform?
  architecture?
  build_id?
  observed_at?
  sample_count?
  lower_percentile_bytes?
  upper_percentile_bytes?
  legacy: boolean
```

All byte arithmetic should use integers. Format only at the presentation layer.
Keep the legacy binary GB convention (`1 GiB = 1,073,741,824 bytes`) unless the
UI explicitly labels decimal units.

### 7.1 User-selectable display modes

For uninstalled games, provide:

- **Depot** — use the Steam depot-manifest estimate.
- **Community** — use the crowdsourced observed estimate.
- **Compare** — display both and a range.

Compare mode should be the transparent default while the data sources are
being validated. Do not silently average or blend the values.

If both sources exist:

```text
size lower bound = min(depot bytes, community representative bytes)
size upper bound = max(depot bytes, community representative bytes)

hours/GB lower bound = hours / size upper bound
hours/GB upper bound = hours / size lower bound
```

The GUI should:

- show both source labels and values;
- visually collapse close values while preserving their provenance;
- flag substantial disagreement, initially suggested at greater than 25%;
- show range-valued cumulative storage in Compare mode; and
- use an explicitly documented rule for sorting interval values, such as the
  conservative hours/GB lower bound.

If only one source is available, show that value with its source. Depot data is
expected to exist for almost every Steam application, but errors and
launcher-only depots must still be representable.

For installed games, the primary displayed and calculated value is the actual
local observation. Depot and community values may still be shown as comparison
data and used to evaluate estimator quality.

## 8. Depot and DLC selection

This is the technically difficult part of the project. Do not estimate a game
by summing every depot listed under its AppID.

### 8.1 Required inputs

- Authenticated account licenses and owned depot access.
- PICS product information for the base application and relevant DLC.
- Current public-branch manifest IDs.
- Target operating system.
- Target CPU architecture.
- Steam/content language.
- Low-violence or regional configuration where applicable.
- DLC ownership and whether each depot is normally mounted/installed.

### 8.2 Selection rules

The implementation should approximate Steam's own depot mounting behavior:

- Include common depots without a conflicting configuration filter.
- Include depots matching the target OS.
- Include depots matching the target architecture, while correctly handling
  depots with no architecture restriction.
- Include the selected language and language-independent content.
- Respect low-violence and other published configuration filters.
- Resolve shared depots and `depotfromapp` relationships.
- Include content depots for DLC the account owns when Steam normally installs
  them with the base application.
- Do not blindly include soundtracks, tools, workshop depots, dedicated
  servers, redistributable packages, optional content, or app-managed DLC.
- Record exclusions and ambiguity as structured warnings for debugging.

Linux needs an explicit policy:

- prefer a native Linux build when Steam would choose it;
- otherwise estimate the Windows depots used through Proton;
- do not add the shared Proton runtime itself to every game;
- do not pretend to know the eventual per-game Proton prefix or shader-cache
  size.

macOS needs correct handling of Intel versus Apple Silicon and any Steam/Rosetta
behavior exposed in product metadata.

### 8.3 Calculate final logical installed bytes, not download bytes

Steam manifests expose compressed chunk sizes and uncompressed file sizes. Only
uncompressed file sizes are relevant.

Simply summing `TotalUncompressedSize` across selected depots can overcount if
depots mount files to the same destination path. For the accurate estimator:

1. retrieve and, where necessary, decrypt the selected manifests;
2. process them in Steam mount order;
3. construct the final mapping of destination path to logical file size;
4. let later mounted depots replace earlier files at the same path; and
5. sum the final logical file sizes.

Do not download content chunks.

The output is a **Steam base install estimate**, not a promise about total
filesystem allocation. It excludes launcher downloads, user content, mods,
workshop files, caches, saves, and filesystem allocation/compression effects.

## 9. Crowdsourced size service

The legacy service is currently called from:

`https://eu5di55p9a.execute-api.eu-west-2.amazonaws.com/default`

Observed legacy client contracts:

- `GET /apps` with a JSON body containing `{"ids": [...]}` in batches of 100;
  returns records containing at least `AppId` and `Size`.
- `GET /app/{appid}` returns a size or not-found response.
- `POST /app/{appid}?size=<bytes>&name=<name>` adds an observation.
- `PUT /app/{appid}?size=<bytes>&name=<name>` updates an observation.

Verify the live Lambda/API implementation before depending on these details.
Do not assume that source code for the Lambda is in the legacy repository.

The current database appears to expose one historical representative size per
AppID. This remains valuable for:

- launcher/bootstrapper games whose depots contain only a small installer;
- games that download content after Steam installation;
- catching errors in depot selection;
- validation and anomaly detection.

It can also become stale after game updates and may mix platforms, languages,
DLC configurations, and builds.

### 9.1 Proposed future community schema

Version the API rather than breaking the legacy client. Prefer storing
observations or aggregate distributions with:

- AppID;
- bytes;
- platform;
- architecture;
- Steam build ID where available;
- observation timestamp;
- language if it materially affects size;
- sample count;
- median and useful percentiles rather than only a mean.

Avoid collecting account identity. Whether contribution is automatic,
first-run consent, or opt-in remains a product decision. The UI must clearly
state exactly what is uploaded. Do not upload owned DLC lists without an
explicit privacy decision; a coarse DLC-present flag or count may be enough for
diagnostics.

## 10. Known estimator failure modes

Design these as visible, testable cases:

- A launcher depot downloads most of the game outside Steam.
- Depot metadata is inaccessible despite ownership.
- A public manifest changes after cached product information.
- Platform filters select both native and compatibility depots.
- Language depots are omitted or double-counted.
- Owned DLC is app-managed or optional rather than automatically installed.
- Shared depots or overlapping destination files are double-counted.
- A game has no meaningful playtime or zero-byte content.
- Community data is stale or mixes incompatible configurations.
- An external Steam library is disconnected.
- Family sharing, free weekends, expired packages, demos, tools, and dedicated
  servers appear in ownership metadata.

Do not hide these behind zero values. Use typed missing/error/warning states.

## 11. Suggested application architecture

Keep domain logic independent of Tauri so it can be tested through a CLI spike
and unit tests.

```text
src/                         frontend
src-tauri/src/
  domain/                    normalized game, size, and source types
  local_steam/               installation and manifest discovery
  steam_auth/                QR/session/token lifecycle
  steam_library/             ownership, playtime, PICS
  depot_selection/           platform/language/license rules
  depot_manifest/            retrieval, merge, and byte calculation
  community/                 versioned AWS client
  commands/                  narrow Tauri IPC commands
spikes/
  steam-protocol/            disposable or promotable feasibility CLI
fixtures/
  public/                    sanitized PICS/manifest fixtures
docs/
  decisions/                 short architecture decision records
```

Frontend framework selection is open. Prefer a small, conventional TypeScript
stack with accessible table controls and minimal state machinery. The domain
model and tests are more important than the framework.

Use structured errors and tracing, with secret redaction. Cache public product
metadata and manifests responsibly, but never cache credentials in the same
store.

## 12. Mandatory phase 0: Steam protocol feasibility spike

The next Codex task should start here.

Build the smallest useful Rust CLI or isolated prototype before scaffolding the
complete Tauri UI. It should:

1. perform Steam QR login;
2. use a non-conflicting logon ID;
3. print only the authenticated SteamID/persona needed to confirm success;
4. retrieve owned game AppIDs, names, and lifetime playtime;
5. retrieve licenses/owned depots and PICS metadata;
6. choose a bounded sample of owned games;
7. resolve current public manifests for the current OS, architecture, and a
   selected/default language;
8. retrieve manifests without downloading chunks;
9. calculate both compressed bytes for diagnostics and final uncompressed
   logical bytes, clearly using only the latter as the install estimate;
10. identify owned DLC depots and report why each depot was included or
    excluded;
11. compare estimates against local `SizeOnDisk` for installed sample games;
12. optionally compare against the legacy community endpoint;
13. log out cleanly; and
14. avoid persisting the refresh token in the first iteration.

The spike should produce a machine-readable report with no secrets, for
example:

```json
{
  "app_id": 620,
  "platform": "macos",
  "architecture": "arm64",
  "selected_depots": [],
  "excluded_depots": [],
  "included_dlc": [],
  "compressed_bytes": 0,
  "uncompressed_bytes": 0,
  "installed_bytes": null,
  "community_bytes": null,
  "warnings": []
}
```

### 12.1 Spike acceptance criteria

Before committing to the protocol implementation, demonstrate:

- QR login without handling a password.
- Owned library and lifetime playtime retrieval.
- No disruption to the normal Steam client session.
- Manifest-only retrieval for owned paid games, not merely anonymous/free
  depots.
- Correct separation of compressed and uncompressed bytes.
- Explainable depot selection for at least:
  - a single-platform game;
  - a multi-platform game;
  - a game with language depots;
  - a game with owned DLC, if available;
  - a game using shared depots, if available.
- Close agreement with local installed sizes for ordinary Steam-hosted games,
  with discrepancies documented rather than massaged.
- A written dependency assessment covering maintenance, licenses, protocol
  coverage, binary impact, and fallback options.

The spike may initially be developed on macOS, but it must not bake macOS paths
or platform values into the domain logic.

### 12.2 Go/no-go decision

At the end, write an architecture decision record choosing one:

1. native Rust Steam protocol implementation;
2. SteamKit2 sidecar;
3. Web API key fallback for library plus another manifest strategy; or
4. stop and reassess because the protocol or security cost is unjustified.

Do not silently proceed with a weak protocol dependency.

## 13. Subsequent phases

### Phase 1 — Tauri foundation and local installed library

- Scaffold Tauri 2.
- Implement cross-platform Steam installation discovery.
- Parse library folders and installed app manifests.
- Show installed games and exact local sizes.
- Establish domain types, fixtures, unit tests, formatting, linting, and CI.

### Phase 2 — Authenticated owned library

- Integrate the selected phase-0 authentication approach.
- Add QR UI, account state, error recovery, logout, and optional secure
  persistence only after a security review.
- Join owned games/playtime with installed observations.

### Phase 3 — Depot estimator

- Implement fully explainable depot selection.
- Add owned DLC handling and file-level manifest merging.
- Cache public metadata.
- Expose calculation diagnostics in development builds.

### Phase 4 — Community comparison

- Integrate the existing API behind a typed provider.
- Implement Depot, Community, and Compare modes.
- Add discrepancy flags and range-valued hours/GB and cumulative storage.
- Propose and, if approved, implement a versioned community observation API.

### Phase 5 — Product UX and distribution

- Sorting, filtering, storage-budget exploration, and source explanations.
- Accessible keyboard and screen-reader behavior.
- Windows, macOS, and Linux packaging.
- Signing/notarization decisions.
- Update mechanism and release CI.
- Privacy documentation.

## 14. Testing strategy

- Unit-test VDF/ACF parsing with sanitized fixtures.
- Unit-test depot filters as pure functions.
- Unit-test manifest path merging and overrides.
- Unit-test bytes and hours/GB interval arithmetic.
- Snapshot sanitized diagnostic reports, never tokens.
- Keep live Steam tests opt-in and excluded from normal CI.
- Create contract tests for the community API with recorded sanitized
  responses.
- Run platform discovery tests on each target OS in CI where possible.
- Manually test external/disconnected library folders.
- Maintain a small documented validation corpus containing ordinary games and
  known launcher/bootstrapper outliers.

## 15. Open product decisions

Do not block the phase-0 spike on these:

- Final product/repository display name.
- Frontend framework.
- Whether persistent Steam login is offered in the first release.
- Default Steam content language and whether it follows the client or a user
  setting.
- Exact Linux native-versus-Proton presentation.
- Community contribution consent/default.
- Community aggregation and outlier policy.
- Whether Compare or Depot mode becomes the long-term default after validation.
- License for the new repository.

## 16. Research notes and empirical sanity check

Steam depot manifests expose both compressed and uncompressed sizes. A small
comparison performed on 2026-07-27 produced:

| Game | Legacy community | Depot uncompressed | Depot compressed | Store requirement |
| --- | ---: | ---: | ---: | ---: |
| Portal 2 | 11.88 GiB | 11.88 GiB | 7.03 GiB | 8 GB |
| Terraria | 444 MiB | 764 MiB | 580 MiB | 200 MB |
| PAYDAY 2 | 86.21 GiB | 84.18 GiB | 34.51 GiB | 83 GB |
| Baldur's Gate 3 | 147.22 GiB | 144.67 GiB | 118.85 GiB | 150 GB |

This is not a sufficient validation set, but it supports three decisions:

- uncompressed manifest bytes, not compressed transfer bytes, are the useful
  depot estimate;
- community observations can become stale; and
- store requirements are too unreliable to include.

No suitable documented third-party install-size API was found. SteamDB exposes
excellent human-facing depot information but explicitly provides no API and
prohibits scraping:
<https://steamdb.info/faq/>.

## 17. Instructions to the next Codex task

1. Read this entire specification before changing files.
2. Inspect the legacy implementation for behavior, but do not modify the legacy
   repository.
3. Check for repository-local `AGENTS.md` or other instructions.
4. Start with section 12, not a polished frontend.
5. Write a short plan and keep the user informed during live Steam testing.
6. Never print or persist Steam credentials or tokens.
7. Prefer evidence from a working spike over assumptions about protocol
   libraries.
8. Record important deviations or decisions under `docs/decisions/`.
9. Stop for user direction if completing a step would require a materially
   broader account permission or privacy policy than specified here.
