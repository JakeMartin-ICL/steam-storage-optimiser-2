# Steam protocol feasibility spike

Date: 2026-07-27

Status: initial login and depot-foundation checks passed

## Decision

Continue with a native Rust implementation behind a small application-owned
interface. The spike uses:

- `steamroom` 0.3 for the Steam client protocol, PICS, depot keys, content
  servers, and depot manifests; and
- `steamroom-client` 0.3 for the high-level QR login flow and manifest response
  decoding.

Both crates are licensed MIT OR Apache-2.0 and come from the same repository.
They cover the complete technical path needed by this product without a
SteamKit2 sidecar or a user-supplied Steam Web API key.

The crates are young `0.x` dependencies. Keep them behind an internal adapter,
pin reviewed versions in `Cargo.lock`, and expect API changes.

## What was proven

A live check on macOS completed successfully while the normal Steam client was
available:

1. The Rust backend opened a TLS WebSocket connection to Steam.
2. Steam returned a QR challenge.
3. Rust rendered the challenge to an SVG data URL, so the challenge text and
   credentials did not enter JavaScript.
4. The official Steam Mobile app scanned and approved the sign-in.
5. The authenticated client retrieved the owned library and lifetime playtime
   through `Player.GetOwnedGames#1`.
6. It retrieved authenticated PICS product metadata for a locally installed
   paid game.
7. It obtained depot decryption keys and manifest request codes.
8. It downloaded depot manifests only, not game content.
9. It parsed compressed and uncompressed manifest sizes and decrypted
   filenames.
10. It compared the locally recorded `SizeOnDisk` with a diagnostic file-level
    merge of the installed depot manifests.
11. A second live check parsed the PICS depot tree, selected current
    public-branch manifests for macOS, ARM64, and English, and retained a reason
    for each selected depot.
12. The selected manifests were merged by normalized destination path. For
    Stick Fight: The Game, depots `674942` and `674944` produced a merged
    uncompressed size of 408,967,389 bytes, exactly matching the local
    `SizeOnDisk`.
13. Steam denied the key for depot `726050`, a converted soundtrack depot. The
    estimator surfaced the denial, omitted that depot, and continued with the
    accessible game depots.
14. A DLC-heavy Civilization V validation resolved account package licences to
    DLC AppIDs and selected all 18 mounted DLC depots. Its 21 merged manifests
    totalled 7,712,266,725 bytes, exactly matching the local `SizeOnDisk`.
15. Steam also supplied borrowed Steam Family licences for two unowned map
    packs. Depot-key access alone incorrectly admitted them. Because the base
    game was directly owned, selecting only direct package entitlements removed
    those two depots while preserving packages that had both borrowed and
    direct licence records. The resulting depot set exactly matched the local
    mount set.
16. `Player.GetOwnedGames#1` returned 663 games for the validation account but
    none of the AppIDs identified as shared-only through package licences, even
    when those AppIDs were supplied explicitly as a filter.
17. The product implementation requested current PICS depot metadata for all
    684 direct and shared-only games in bounded batches. On the validation
    account it produced 516 same-platform estimates and 163 explicitly labelled
    Windows fallbacks in 39.2 seconds. Only five games remained unavailable.
18. Batman: Arkham Knight exposed the platform false-positive that motivated
    the base-depot rule: platform-neutral DLC matched macOS despite the base
    game being Windows-only. The corrected selection used ten Windows depots
    totalling 59,450,251,011 bytes and marked the game unavailable on macOS.
19. Grounded exposed the same class of bug through a tiny platform-neutral
    helper depot rather than DLC. Requiring an OS-specific base depot whenever
    the app defines OS-specific base content produced a four-depot Windows
    estimate of 12,144,896,824 bytes and corrected 23 additional compatibility
    classifications.
20. Steam can split a large PICS product-info request across response messages,
    while `steamroom` 0.3 returns the first response. Ten-app batches plus
    individual recovery for any omitted AppID avoided false missing-metadata
    results without modifying the dependency.
21. PICS classified the 27 borrowed-only AppID candidates as 21 games and six
    non-game entries. Merging those game records produced a 684-game library
    with 21 records marked `sharedOnly`.
22. The local account's `localconfig.vdf` supplied playtime for a shared-only
    game omitted by `GetOwnedGames`. Shared records without a local playtime
    entry are treated as unplayed, while the recorded value is preserved for
    games such as Paralives.
23. Successful depot summaries are cached locally for 24 hours. A live
    validation restored 682 estimates and retried only two unavailable games
    in about 0.5 seconds, compared with roughly 41 seconds to populate the
    cache. Cache reuse requires the same Steam account, package-entitlement
    fingerprint, platform, architecture, and game language.

The user confirmed that the app reached its successful result screen. No
account identifiers, tokens, or private fixtures were written to the
repository.

## Security behavior

- The app never asks for a Steam account name, password, SteamID, Web API key,
  or Steam Guard code.
- Access and refresh tokens stay within the native Rust backend.
- The access token is decoded in Rust only to identify the authenticated
  SteamID for the owned-games request.
- For development only, the refresh token is cached in a private local
  application-data file with user-only permissions so repeated live protocol
  checks do not require another QR approval. This temporary spike behavior must
  not ship unchanged.
- Cancelling aborts the task and drops the connection.
- Completing the check drops the authenticated client after the data is read.
- The webview receives a generated QR image and sanitized result data, never
  token text.
- Errors shown to the UI are reduced to one bounded line.

Production persistent login must be explicit, use an appropriate operating
system credential store, and include token revocation/removal on logout.

## Depot foundation now implemented

The estimator no longer relies on manifest IDs recorded in the local app
manifest. It now:

- parses typed depot configuration and public-branch manifest references from
  live PICS metadata;
- selects by operating system, CPU architecture, language, optional-content
  marker, and owned DLC AppID;
- resolves active direct and Steam Family package licences through PICS and
  keeps their lender provenance;
- uses direct DLC when the base game is directly owned, or DLC from Steam's
  preferred/sole lender when the base game is shared-only;
- follows `depotfromapp` when requesting keys and manifests;
- records shared-install, low-violence, and architecture ambiguities;
- excludes recognizable soundtrack, dedicated-server, redistributable, and
  workshop depot names;
- treats an inaccessible individual depot as an explicit omission rather than
  turning the whole estimate into zero or aborting other valid depots;
- ignores zero-byte structural depots as installable content or evidence of
  operating-system compatibility;
- reports why each successfully read depot was selected; and
- merges uncompressed manifest files by normalized destination path to avoid
  double-counting overlaps.

The product uses two explicit depot precision levels:

- a fast library-wide estimate adds Steam's current uncompressed public
  manifest summaries for the selected platform/language/DLC depots; and
- a file-level refinement downloads only manifest metadata, decrypts filenames,
  and merges destination paths to remove overlap.

The fast result is useful for broad coverage but may over-count games whose
selected depots contain the same destination paths. The UI labels it as a
manifest summary and explains this caveat; it never labels the value as an
exact local installation size.

Platform support is established by a selected base-game depot, not merely any
selected depot. If the app defines OS-specific base depots, one of those must
match; platform-neutral DLC, blank depots, and helper depots cannot establish
compatibility by themselves. When no base-game depot matches the current
operating system, the estimator deliberately selects the Windows depot set
instead and labels both the game and size source as Windows.

The installed app manifest is used only to choose a live validation title,
obtain its configured language, and compare the resulting estimate with
`SizeOnDisk`.

## What remains unproven

This is now a working estimator foundation, but the validation sample is still
too small for production confidence. Remaining work includes:

- determine the authoritative Steam mount/override order when overlapping
  depots contain the same path with different sizes;
- improve content-role classification when PICS supplies no useful depot name
  or optional marker (the live soundtrack depot was caught by key denial);
- model Linux native-versus-Proton choices and validate macOS architecture
  behavior beyond Steam's generic `64` filter;
- validate ordinary, multi-language, owned-DLC, launcher/bootstrapper, and
  disconnected-library cases across more installed games; and
- validate the `PreferredOwner` licence flag against an installed shared-only
  game and decide how the product should present multiple unresolved lenders;
- validate how completely Steam synchronizes local `Playtime` values across
  devices and support hiding `sharedOnly` games in the product UI;
- compare estimates with the unchanged community endpoint.

The exact match in the first metadata-driven live case is encouraging, not a
claim of general correctness. Ambiguity remains visible to the user.

## Testing strategy

The initial automated suite covers:

- local VDF/app-manifest parsing with sanitized data;
- QR image generation without exposing the challenge URL in the data URL;
- selection of a valid installed probe target;
- typed PICS depot parsing with sanitized metadata;
- platform, architecture, language, optional-content, DLC, shared-depot, and
  soundtrack selection rules;
- active-versus-expired and direct-versus-borrowed package modelling, including
  preferred-lender selection and ambiguous-lender behavior;
- overlap-aware uncompressed-size arithmetic;
- the React login entry point and native-command boundary; and
- production TypeScript and Tauri compilation.

Live account checks stay explicit and local because they require mobile
approval and real entitlements. They must not be recorded as fixtures.

## Recommendation

Proceed with the native Rust route. Do not build the API-key fallback at this
stage. The code now has separate authentication, library, local-Steam,
depot-metadata, depot-selection, and depot-probe boundaries. Expand the
validation matrix before treating the depot figure as a product-wide estimate.
