# Community size compatibility

Date: 2026-07-27

## Legacy contract

The v2 read adapter preserves the behavior observed in the legacy Python
client:

- base URL:
  `https://eu5di55p9a.execute-api.eu-west-2.amazonaws.com/default`;
- batch lookup: `GET /apps` with a JSON body shaped as
  `{"ids": [<AppID>, ...]}`;
- maximum batch size: 100 AppIDs; and
- response: a JSON array containing PascalCase `AppId`, `Size`, and optional
  `Name` fields.

The v2 client also preserves the legacy contribution contract:

- `POST /app/{appid}?size=<bytes>&name=<name>` creates a missing observation;
- `PUT /app/{appid}?size=<bytes>&name=<name>` contributes when the current
  community value differs by more than 1 GiB; and
- an enabled-by-default login preference lets the user opt out before any
  contribution is attempted.

The app stores the last observed local size for each AppID in local application
data. A game is reconsidered only when its local `SizeOnDisk` differs from that
baseline by at least 100 MiB. Smaller changes do not advance the baseline, so
they must accumulate to the threshold and restarting the app cannot repeatedly
weight the community average. Depot estimates never enter this cache or the
contribution path. Failed requests are not recorded and can be retried later.

The response parser is strict about the established shape so a backend
contract change becomes a visible error rather than silently producing missing
or zero sizes. Requests use the existing unusual GET-with-JSON-body behavior
for compatibility, run in batches of 100, and have a 15-second timeout.

## Presentation rules

The calculation layer implements the product rules without averaging the two
sources:

- local `SizeOnDisk` is authoritative whenever the game is installed;
- Depot mode prefers the depot estimate and explicitly falls back to Community;
- Community mode prefers the community estimate and explicitly falls back to
  Depot;
- Compare mode collapses close observations to the depot value when the
  absolute difference is at most 100 MiB or the community value differs from
  the depot value by at most 15%;
- larger disagreements retain both values and report the low-to-high size
  range;
- the hours/GB range is reversed relative to size:
  `hours / largest size ... hours / smallest size`; and
- missing and zero values remain unavailable rather than becoming zero
  efficiency.

The UI identifies collapsed observations as a close match. True ranges use an
arrow and share a binary unit only when both endpoints naturally use that same
unit. The hours/GB bar renders the conservative value in the primary
blue-to-green colour and appends the remaining range in violet, making source
disagreement visible without reading the figures.

## Privacy and live verification

A batch read reveals a set of public AppIDs to the existing community service.
A contribution contains the public AppID, game name, and observed installation
size. It does not contain a SteamID, profile, playtime, account identifier, or
other personal data. The login UI explains this and provides a contribution
opt-out; community estimate reads remain available when contributions are
disabled.

Automated tests use sanitized response fixtures. No live library AppIDs or
account data are stored in the repository.
