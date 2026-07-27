# Steam Storage Optimiser

Steam Storage Optimiser is a desktop app for deciding which games deserve
space on your drive. It combines lifetime playtime with local or estimated
installation sizes, then shows hours played per gigabyte across your Steam
library.

The app is the Rust/Tauri successor to the original
[Python project](https://github.com/JakeMartin-ICL/steam-storage-optimiser).

## Features

- Sign in by scanning Steam's QR code—no Steam Web API key required.
- Browse owned and Steam Family games with playtime, artwork, platform
  compatibility, and shared-library status.
- Read exact `SizeOnDisk` values for locally installed games.
- Estimate uninstalled games directly from Steam depot manifests.
- Compare Steam depot estimates with the existing crowdsourced size database.
- Filter, search, and sort by storage, playtime, or hours played per gigabyte.
- Set a library-size target and see how each game changes the cumulative total.
- Hide shared-only or current-OS-incompatible games.
- Inspect depot selection and source details for individual games.

## How sizes are calculated

Installed games always use Steam's local `SizeOnDisk` value.

For uninstalled games, the default Depot mode selects public Steam manifests
matching the relevant operating system, architecture, language, DLC ownership,
and Steam Family licence. If no compatible current-OS base depot exists, the
app uses a clearly labelled Windows estimate. Successful depot results are
cached for 24 hours.

Community mode uses the crowdsourced database maintained for the original
project. Compare mode keeps meaningful disagreements as a range and collapses
close results to the Steam depot value. Large discrepancies are flagged because
depot estimates can omit launcher or bootstrap files, while community
observations can be old or come from another operating system.

The cumulative column compares the visible library with a configurable storage
target. Its initial suggestion is half the capacity of the filesystem
containing the primary Steam installation; if that cannot be detected, it
defaults to 1 TiB.

## Privacy and local data

Steam authentication, local library discovery, and size calculations run in
the desktop app. Community-size contributions are enabled by default but can be
disabled before login. A contribution contains only a public Steam AppID, game
name, and observed local installation size—never the user's SteamID, profile,
or playtime.

The app records the last contributed local size and only reconsiders a game
after its `SizeOnDisk` changes by at least 100 MiB. Depot estimates never enter
the contribution path.

The current development build stores the Steam refresh token in the per-user
local application-data directory, with explicit user-only file permissions on
Unix systems. This convenience cache should be replaced with production
credential storage before release.

## Development

The live Steam login, library, family ownership, community-size, and depot
flows have been validated on macOS. Other desktop targets still need platform
validation.

Prerequisites:

- Node.js and npm
- a current stable Rust toolchain
- the platform dependencies required by Tauri 2

Install dependencies and start the desktop app:

```sh
npm install
npm run tauri dev
```

Create a production bundle:

```sh
npm run tauri build
```

## Releases

Pushing a version tag builds a draft GitHub Release containing:

- an x64 NSIS installer (`.exe`) for Windows;
- separate Apple Silicon and Intel disk images (`.dmg`) for macOS; and
- an x64 AppImage plus Debian package (`.deb`) for Linux.

The tag must match the version in `src-tauri/tauri.conf.json`. For example:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The initial macOS artifacts use an ad-hoc signature. Windows and macOS
production signing can be added to the release workflow through repository
secrets before publishing the draft.

## Installation

Download the latest bundle for your operating system from the project's GitHub
Releases page.

### Windows

Download and run the `.exe` installer. Windows may show a SmartScreen warning
until production code signing is configured.

### macOS

Choose the Apple Silicon DMG for Macs with an M-series processor, or the Intel
DMG for older Macs. Open the `.dmg` and drag Steam Storage Optimiser into the
Applications folder.

The current builds are ad-hoc signed rather than Apple-notarized. On first
launch, macOS may require you to open **System Settings → Privacy & Security**
and choose **Open Anyway**.

### Linux

The AppImage runs without installation on most distributions:

```sh
chmod +x Steam.Storage.Optimiser_*.AppImage
./Steam.Storage.Optimiser_*.AppImage
```

On Debian, Ubuntu, and compatible distributions, install the `.deb` package
instead:

```sh
sudo apt install ./steam-storage-optimiser_*.deb
```

Run all automated checks:

```sh
npm test
npm run build
cd src-tauri
cargo test
cargo clippy --all-targets -- -D warnings
```

The frontend is React and TypeScript. Steam integration, local discovery,
caching, community compatibility, and depot selection are implemented in Rust.
Technical findings are documented in
[`docs/feasibility-spike.md`](docs/feasibility-spike.md) and
[`docs/community-size-source.md`](docs/community-size-source.md).
