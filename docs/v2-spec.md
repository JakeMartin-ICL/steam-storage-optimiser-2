# Steam Storage Optimiser v2 — Project Brief

Status: handoff brief

Date: 2026-07-27

Legacy repository: <https://github.com/JakeMartin-ICL/steam-storage-optimiser>

Legacy local path at handoff:
`/Users/jakemartin/Projects/steam-storage-optimiser`

## 1. What this project is

Steam Storage Optimiser helps users decide which games deserve disk space. Its
main metric is **lifetime hours played per GB of installed size**.

The legacy Python terminal application:

- retrieves the user's owned games and lifetime playtime through the Steam Web
  API;
- finds installed games through Steam's local library and app manifests;
- uses each installed app manifest's `SizeOnDisk`;
- estimates uninstalled games through an existing crowdsourced AWS service;
- contributes observed installed sizes to that service; and
- displays a table sorted by hours per GB.

This repository is a clean successor rather than a migration. Preserve the
legacy repository as a working historical project with its Python packaging and
release setup.

The intended v2 is a local-first Tauri 2 desktop application for Windows,
macOS, and Linux. It should offer a useful GUI, discover local Steam data, and
reduce the setup required from the user.

## 2. Product decisions

- The metric remains hours played per GB. Price is not involved.
- Use Tauri 2 and target Windows, macOS, and Linux.
- Installed games use the local Steam app manifest's `SizeOnDisk`.
- Uninstalled games should use both Steam depot estimates and the existing
  crowdsourced estimate when available.
- Do not use Steam store system requirements as a size source.
- Let users choose between depot and community values, and offer a comparison
  or range view. Do not silently average the two.
- Depot estimates must use uncompressed installed-file sizes, not compressed
  download sizes.
- Include owned DLC that Steam would normally install with the game when this
  can be determined reliably.
- Keep the existing crowdsourcing Lambda and its API unchanged. Adapting,
  migrating, or replacing that backend is outside the scope of v2.

## 3. Desired user experience

The ideal application:

1. finds the local Steam installation and library folders;
2. signs the user in, preferably without asking for a Web API key or SteamID;
3. retrieves owned games and lifetime playtime;
4. shows exact local sizes for installed games;
5. estimates uninstalled games from Steam depots;
6. also shows the existing community estimate where one exists; and
7. makes uncertainty and disagreement between sources understandable.

It must not install, uninstall, move, or modify games. Reading Steam's files and
retrieving metadata should be enough.

## 4. Authentication direction

The preferred hypothesis is a fully local Steam client-protocol login, ideally
using QR approval through the official Steam Mobile app. This may allow the app
to retrieve the owned library, lifetime playtime, licenses, product metadata,
and depot manifests without a backend or user-supplied API key.

This needs a bounded feasibility investigation before committing the product to
it. Native Rust libraries may work, while SteamKit2 in a sidecar is a possible
fallback. The next agent should research the current options and choose based on
working evidence, security, maintenance, licensing, and packaging impact rather
than treating any library named here as mandatory.

If local protocol login is impractical, the pragmatic fallback is:

- detect SteamID values from Steam's local `config/loginusers.vdf`; and
- ask the user only for their Steam Web API key.

This still halves the legacy setup. OpenID plus a centrally held API key is
possible but adds backend maintenance and is not preferred.

Security constraints:

- Never ask for, handle, or store a Steam password.
- Never log or persist tokens or API keys in plaintext.
- Keep credentials out of the webview.
- Avoid disrupting the user's normal Steam client session.
- If persistent login is eventually offered, make it explicit and use the
  operating system credential store.
- Provide a real logout that removes retained credentials.

## 5. Local Steam data

Use Steam's own files instead of scanning arbitrary disks:

- `config/loginusers.vdf` for known local accounts;
- `steamapps/libraryfolders.vdf` for library locations; and
- `steamapps/appmanifest_<appid>.acf` for installed state and `SizeOnDisk`.

Discovery must account for conventional Steam locations on each target
platform and tolerate missing or disconnected external libraries. Filesystem
access should be read-only and narrowly scoped.

## 6. Size sources and presentation

### Installed games

The local `SizeOnDisk` observation is authoritative for the current
installation. Depot and community figures can still be shown for comparison.

### Uninstalled games

Use two independent estimates where possible:

- **Depot**: the logical uncompressed size derived from the Steam depots that
  would be installed for the user's platform and configuration.
- **Community**: the existing historical installed-size value returned by the
  crowdsourcing API.

The community value can catch launcher or bootstrapper games whose Steam depot
is only a small installer. It can also be stale, or reflect a different
platform, DLC set, language, or game version. The depot value is broader in
coverage but can be wrong if depot selection is wrong or the game downloads
content outside Steam.

The UI should support:

- Depot mode;
- Community mode, with a clear fallback when no value exists; and
- Compare mode, showing both values or the range between them.

For a range, hours-per-GB runs in the opposite direction to size:

```text
size range = min(depot, community) ... max(depot, community)
hours/GB range = hours / max(size) ... hours / min(size)
```

The exact default mode, sorting behavior, disagreement threshold, and visual
design are product choices to make during implementation.

## 7. Depot estimation

Depot selection is the main technical risk. A correct estimate cannot simply
sum every depot associated with an AppID.

The estimator should approximate the content Steam would install for this user,
considering where the metadata permits:

- operating system and CPU architecture;
- language and other configuration filters;
- current public-branch manifests;
- common and shared depots;
- depot relationships with other apps;
- account licenses; and
- owned DLC that Steam normally installs with the base game.

It should avoid unrelated or optional content such as soundtracks, dedicated
servers, tools, workshop content, and redistributables. Linux native-versus-
Proton behavior and macOS architecture behavior need investigation rather than
hard-coded assumptions.

Steam manifests distinguish compressed transfer size from uncompressed file
size. Only the latter is relevant here. Selected depots may also contain
overlapping destination paths, so the investigation should determine whether
file-level merging or mount-order handling is necessary to avoid double
counting.

Do not download game content. The result is a **Steam-managed base installation
estimate**, not total disk usage including launcher downloads, mods, workshop
files, saves, shader caches, Proton prefixes, or filesystem allocation effects.

Keep enough provenance to explain which depots and DLC were included and to
surface ambiguity rather than converting failures to zero.

## 8. Existing community service

The legacy client currently uses:

`https://eu5di55p9a.execute-api.eu-west-2.amazonaws.com/default`

Observed contracts in the legacy code include:

- `GET /apps` with `{"ids": [...]}` batches;
- `GET /app/{appid}`;
- `POST /app/{appid}?size=<bytes>&name=<name>`; and
- `PUT /app/{appid}?size=<bytes>&name=<name>`.

Inspect the legacy client to confirm its exact behavior and preserve
compatibility. The Lambda, database, API shape, and aggregation behavior are
not part of this project and should not be changed.

The service's limited accuracy is acceptable: it is now a secondary comparison
and anomaly signal rather than the only estimate for uninstalled games. The UI
should not imply that it is platform-, build-, or DLC-specific when the
existing response does not provide that information.

## 9. First step: feasibility spike

Before building the full UI, answer the risky Steam-protocol and depot questions
with the smallest useful prototype. The form of the prototype and its
implementation are intentionally left to the next agent.

The investigation should establish whether a distributable local application
can:

- authenticate safely without collecting a password;
- retrieve owned games and lifetime playtime;
- coexist with the normal Steam client;
- access the metadata and manifests needed for owned paid games;
- distinguish compressed from uncompressed sizes;
- make explainable platform, language, and DLC depot selections; and
- produce depot estimates that broadly agree with `SizeOnDisk` for ordinary
  installed games while exposing outliers.

Use a small representative sample rather than attempting exhaustive coverage.
Compare depot estimates with local installed sizes and, optionally, the
unchanged community endpoint. Do not expose credentials in output or fixtures.

Record the findings and recommend one of:

- a native Rust protocol implementation;
- a SteamKit2 or other contained sidecar;
- the Web API key fallback with automatic SteamID discovery; or
- a revised approach if the protocol/security cost is not justified.

This is a decision point, not a demand to prove a predetermined architecture.

## 10. Useful validation cases

As development proceeds, cover a mixture of:

- ordinary single-platform games;
- multi-platform and language-specific depots;
- owned DLC;
- shared or overlapping depots;
- launcher/bootstrapper games;
- external or disconnected library folders;
- zero-playtime and missing-size cases; and
- community values that significantly disagree with local or depot sizes.

Automated tests should focus on pure parsing, depot-selection rules, size
arithmetic, and sanitized recorded metadata. Live account tests should remain
explicit and local.

## 11. Prior empirical check

A small comparison performed on 2026-07-27 found:

| Game | Legacy community | Depot uncompressed | Depot compressed | Store requirement |
| --- | ---: | ---: | ---: | ---: |
| Portal 2 | 11.88 GiB | 11.88 GiB | 7.03 GiB | 8 GB |
| Terraria | 444 MiB | 764 MiB | 580 MiB | 200 MB |
| PAYDAY 2 | 86.21 GiB | 84.18 GiB | 34.51 GiB | 83 GB |
| Baldur's Gate 3 | 147.22 GiB | 144.67 GiB | 118.85 GiB | 150 GB |

This is not a sufficient validation set, but it supports using uncompressed
manifest bytes, retaining community observations as a second signal, and
excluding store requirements.

No suitable documented third-party install-size API was found. SteamDB exposes
useful human-facing depot information but provides no API and prohibits
scraping: <https://steamdb.info/faq/>.

## 12. Guidance for the next Codex task

Read this brief and inspect the legacy implementation for behavioral context,
without modifying the legacy repository. Begin with the feasibility work in
section 9, share findings with the user, and let evidence shape the
architecture. Preserve the product decisions and safety constraints above, but
otherwise treat implementation structure, libraries, frontend framework,
milestones, and detailed UX as open design work.
