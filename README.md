# Steam Storage Optimiser

Steam Storage Optimiser is a desktop app for deciding which games deserve
space on your drive. It combines lifetime playtime with local or estimated
installation sizes, then shows hours played per gigabyte across your Steam
library. Using HowLongToBeat, it can also show an estimate of remaining hours
of play per gigabyte.

<img width="1507" height="695" alt="image" src="https://github.com/user-attachments/assets/c651addb-496d-4c2c-89bc-df8907175152" />

The app is the Rust/Tauri successor to the original
[Python project](https://github.com/JakeMartin-ICL/steam-storage-optimiser).

## Features

- Sign in by scanning Steam's QR code.
- Read exact `SizeOnDisk` values for locally installed games.
- Find standard Steam installations and libraries on additional drives, with a
  manual folder picker for nonstandard locations.
- Estimate size of games not installed from Steam depot manifests and a crowdsourced
  database.
- Compare hours played and estimated hours remaining per gigabyte using
  HowLongToBeat's Main Story, Main + Extras, or Completionist times.
- Set a library-size target and see how each game changes the cumulative total.
  Eg: Set 1TB, sort by hours-remaining/GB, install all games above the point where
  cumulative total exceeds 1TB.

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

## Steam installation discovery

If automatic discovery misses a custom installation, use **Locate Steam** and
select either the main Steam folder or its `steamapps` directory. The selection
is saved for future launches and can be changed from the account panel.

## Privacy and local data

Steam authentication, local library discovery, and size calculations run in
the desktop app. Community-size contributions are enabled by default but can be
disabled before login. A contribution contains only a public Steam AppID, game
name, and observed local installation size—never the user's SteamID, profile,
or playtime.

HowLongToBeat matches are cached locally for about six months because completion
times change infrequently. The initial fetch can take a while for large libraries.

## Development

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

