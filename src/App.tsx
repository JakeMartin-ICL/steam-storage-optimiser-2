import { invoke } from "@tauri-apps/api/core";
import { type CSSProperties, useEffect, useMemo, useState } from "react";
import "./App.css";

let autoResumeAttempted = false;
const contributionPreferenceKey = "contribute-community-sizes";
const storageTargetPreferenceKey = "storage-target-bytes";

export type LibraryGame = {
  appId: number;
  name: string;
  playtimeMinutes: number | null;
  sharedOnly: boolean;
  installed: boolean;
  localSizeBytes: number | null;
  depotSizeBytes: number | null;
  depotStatus: "pending" | "available" | "unavailable";
  depotExact: boolean;
  depotCount: number | null;
  depotOs: string | null;
  currentOsSupported: boolean | null;
  depotWarnings: string[];
  depotError: string | null;
  communitySizeBytes: number | null;
};

export type DepotResult = {
  depotId: number;
  manifestId: number;
  compressedBytes: number;
  uncompressedBytes: number;
  fileCount: number;
  selectionReasons: string[];
};

export type DepotProbe = {
  appId: number;
  appName: string;
  localSizeBytes: number;
  productMetadataBytes: number;
  mergedManifestBytes: number;
  selectionWarnings: string[];
  depots: DepotResult[];
};

export type AuthView = {
  phase: string;
  message: string;
  qrImage: string | null;
  libraryCount: number | null;
  games: LibraryGame[];
  probe: DepotProbe | null;
  error: string | null;
  savedLogin: boolean;
  communityError: string | null;
  depotError: string | null;
  depotProgress: DepotProgress | null;
  profile: SteamProfile | null;
};

export type SteamProfile = {
  displayName: string;
  avatarUrl: string | null;
};

export type StorageTargetDefault = {
  targetBytes: number;
  filesystemSizeBytes: number;
};

export type DepotProgress = {
  completed: number;
  total: number;
  available: number;
  unavailable: number;
};

export const initialAuthView: AuthView = {
  phase: "idle",
  message: "Connect Steam to build your storage-aware library.",
  qrImage: null,
  libraryCount: null,
  games: [],
  probe: null,
  error: null,
  savedLogin: false,
  communityError: null,
  depotError: null,
  depotProgress: null,
  profile: null,
};

export type SourceMode = "depot" | "community" | "compare";
export type LibraryScope = "all" | "installed" | "uninstalled";
export type SortMode = "efficiency" | "playtime" | "size" | "name";

export type GameMetrics = {
  lowerSizeBytes: number;
  upperSizeBytes: number;
  lowerHoursPerGiB: number | null;
  upperHoursPerGiB: number | null;
  sources: Array<"local" | "depot" | "community">;
  fallback: boolean;
  collapsedComparison: boolean;
} | null;

export type CumulativeSize = {
  lowerSizeBytes: number;
  upperSizeBytes: number;
  unknownCount: number;
};

export type EfficiencyBarWidths = {
  basePercent: number;
  rangePercent: number;
};

export type StorageBarWidths = {
  basePercent: number;
  rangePercent: number;
  capped: boolean;
};

export type CumulativeBarWidths = {
  previousPercent: number;
  addedPercent: number;
  overTarget: boolean;
};

const gib = 1024 ** 3;
const tib = 1024 * gib;
const storageTargetMinimum = 10 * gib;
const comparisonAbsoluteTolerance = 100 * 1024 ** 2;
const comparisonRelativeTolerance = 0.15;
const discrepancyAbsoluteThreshold = 100 * 1024 ** 2;
const discrepancyRelativeThreshold = 0.66;

export function formatBytes(bytes: number) {
  if (bytes === 0) return "0 B";
  const { divisor, exponent, unit } = storageUnit(bytes);
  return `${(bytes / divisor).toFixed(exponent > 1 ? 2 : 0)} ${unit}`;
}

function formatStorageTarget(bytes: number) {
  if (bytes < tib) return `${Math.round(bytes / gib)} GiB`;
  return `${(bytes / tib).toFixed(2)} TiB`;
}

export function getGameMetrics(
  game: LibraryGame,
  mode: SourceMode,
): GameMetrics {
  const calculate = (
    lower: number,
    upper: number,
    sources: Array<"local" | "depot" | "community">,
    fallback: boolean,
    collapsedComparison = false,
  ): NonNullable<GameMetrics> => {
    const hours =
      game.playtimeMinutes === null ? null : game.playtimeMinutes / 60;
    return {
      lowerSizeBytes: lower,
      upperSizeBytes: upper,
      lowerHoursPerGiB: hours === null ? null : hours / (upper / gib),
      upperHoursPerGiB: hours === null ? null : hours / (lower / gib),
      sources,
      fallback,
      collapsedComparison,
    };
  };

  if (game.localSizeBytes && game.localSizeBytes > 0) {
    return calculate(
      game.localSizeBytes,
      game.localSizeBytes,
      ["local"],
      false,
    );
  }

  const depot = game.depotSizeBytes || null;
  const community = game.communitySizeBytes || null;
  if (mode === "depot") {
    if (depot) return calculate(depot, depot, ["depot"], false);
    if (community)
      return calculate(community, community, ["community"], true);
  }
  if (mode === "community") {
    if (community)
      return calculate(community, community, ["community"], false);
    if (depot) return calculate(depot, depot, ["depot"], true);
  }
  if (mode === "compare") {
    if (depot && community) {
      const difference = Math.abs(depot - community);
      const sourcesAreClose =
        difference <= comparisonAbsoluteTolerance ||
        difference / depot <= comparisonRelativeTolerance;
      if (sourcesAreClose) {
        return calculate(
          depot,
          depot,
          ["depot", "community"],
          false,
          true,
        );
      }
      return calculate(
        Math.min(depot, community),
        Math.max(depot, community),
        ["depot", "community"],
        false,
      );
    }
    if (depot) return calculate(depot, depot, ["depot"], true);
    if (community)
      return calculate(community, community, ["community"], true);
  }
  return null;
}

export function hasLargeSizeDiscrepancy(game: LibraryGame) {
  const depot = game.depotSizeBytes;
  const community = game.communitySizeBytes;
  if (!depot || !community) return false;
  const difference = Math.abs(depot - community);
  return (
    difference >= discrepancyAbsoluteThreshold &&
    difference / Math.min(depot, community) > discrepancyRelativeThreshold
  );
}

export function filterAndSortGames(
  games: LibraryGame[],
  options: {
    query: string;
    scope: LibraryScope;
    hideShared: boolean;
    hideIncompatible: boolean;
    sort: SortMode;
    sourceMode: SourceMode;
  },
) {
  const query = options.query.trim().toLocaleLowerCase();
  return games
    .filter((game) => {
      if (query && !game.name.toLocaleLowerCase().includes(query)) return false;
      if (options.hideShared && game.sharedOnly) return false;
      if (options.hideIncompatible && game.currentOsSupported === false)
        return false;
      if (options.scope === "installed" && !game.installed) return false;
      if (options.scope === "uninstalled" && game.installed) return false;
      return true;
    })
    .sort((left, right) => {
      const leftMetric = getGameMetrics(left, options.sourceMode);
      const rightMetric = getGameMetrics(right, options.sourceMode);
      if (options.sort === "name") return left.name.localeCompare(right.name);
      if (options.sort === "playtime") {
        return (
          (right.playtimeMinutes ?? -1) - (left.playtimeMinutes ?? -1) ||
          left.name.localeCompare(right.name)
        );
      }
      if (options.sort === "size") {
        return (
          (rightMetric?.upperSizeBytes ?? -1) -
            (leftMetric?.upperSizeBytes ?? -1) ||
          left.name.localeCompare(right.name)
        );
      }
      return (
        (rightMetric?.lowerHoursPerGiB ?? -1) -
          (leftMetric?.lowerHoursPerGiB ?? -1) ||
        left.name.localeCompare(right.name)
      );
    });
}

export function buildCumulativeSizes(
  games: LibraryGame[],
  sourceMode: SourceMode,
): CumulativeSize[] {
  let lowerSizeBytes = 0;
  let upperSizeBytes = 0;
  let unknownCount = 0;
  return games.map((game) => {
    const metrics = getGameMetrics(game, sourceMode);
    if (metrics) {
      lowerSizeBytes += metrics.lowerSizeBytes;
      upperSizeBytes += metrics.upperSizeBytes;
    } else {
      unknownCount += 1;
    }
    return { lowerSizeBytes, upperSizeBytes, unknownCount };
  });
}

export function getEfficiencyBarWidths(
  metrics: GameMetrics,
): EfficiencyBarWidths | null {
  if (!metrics || metrics.lowerHoursPerGiB === null) return null;
  const upperHours = metrics.upperHoursPerGiB ?? metrics.lowerHoursPerGiB;
  const basePercent = Math.min(
    Math.max(metrics.lowerHoursPerGiB * 3, 0),
    100,
  );
  const upperPercent = Math.min(Math.max(upperHours * 3, 0), 100);
  return {
    basePercent,
    rangePercent: Math.max(upperPercent - basePercent, 0),
  };
}

export function getStorageBarWidths(
  metrics: GameMetrics,
  maximumSizeBytes: number,
): StorageBarWidths | null {
  if (!metrics || maximumSizeBytes <= 0) return null;
  const basePercent = Math.min(
    Math.max((metrics.lowerSizeBytes / maximumSizeBytes) * 100, 0),
    100,
  );
  const upperPercent = Math.min(
    Math.max((metrics.upperSizeBytes / maximumSizeBytes) * 100, 0),
    100,
  );
  return {
    basePercent,
    rangePercent: Math.max(upperPercent - basePercent, 0),
    capped: metrics.upperSizeBytes > maximumSizeBytes,
  };
}

function quantile(sortedValues: number[], fraction: number) {
  if (sortedValues.length === 1) return sortedValues[0];
  const position = (sortedValues.length - 1) * fraction;
  const lowerIndex = Math.floor(position);
  const upperIndex = Math.ceil(position);
  const lower = sortedValues[lowerIndex];
  const upper = sortedValues[upperIndex];
  return lower + (upper - lower) * (position - lowerIndex);
}

export function getStorageScaleMaximum(sizes: number[]) {
  const sorted = sizes
    .filter((size) => Number.isFinite(size) && size > 0)
    .sort((left, right) => left - right);
  if (sorted.length === 0) return 0;
  const maximum = sorted[sorted.length - 1];
  const lowerQuartile = quantile(sorted, 0.25);
  const upperQuartile = quantile(sorted, 0.75);
  const ninetyFifthPercentile = quantile(sorted, 0.95);
  const outlierFence =
    upperQuartile + 1.5 * (upperQuartile - lowerQuartile);
  return Math.min(
    maximum,
    Math.max(ninetyFifthPercentile, outlierFence),
  );
}

export function getCumulativeBarWidths(
  cumulative: CumulativeSize,
  addedSizeBytes: number,
  targetSizeBytes: number,
  totalCumulativeSizeBytes: number,
): CumulativeBarWidths | null {
  if (targetSizeBytes <= 0) return null;
  const overTarget = cumulative.lowerSizeBytes > targetSizeBytes;
  const scaleSizeBytes = overTarget
    ? Math.max(totalCumulativeSizeBytes, 1)
    : targetSizeBytes;
  const previousSizeBytes = Math.max(
    cumulative.lowerSizeBytes - addedSizeBytes,
    0,
  );
  const previousPercent = Math.min(
    (previousSizeBytes / scaleSizeBytes) * 100,
    100,
  );
  const addedPercent = Math.min(
    (addedSizeBytes / scaleSizeBytes) * 100,
    100 - previousPercent,
  );
  return { previousPercent, addedPercent, overTarget };
}

function App() {
  const [auth, setAuth] = useState<AuthView>(initialAuthView);
  const [commandError, setCommandError] = useState<string | null>(null);
  const [contributeCommunitySizes, setContributeCommunitySizes] = useState(
    () => {
      try {
        return window.localStorage.getItem(contributionPreferenceKey) !== "false";
      } catch {
        return true;
      }
    },
  );

  useEffect(() => {
    try {
      window.localStorage.setItem(
        contributionPreferenceKey,
        String(contributeCommunitySizes),
      );
    } catch {
      // A restricted webview can still use the in-memory default.
    }
  }, [contributeCommunitySizes]);

  useEffect(() => {
    let disposed = false;
    const refresh = async () => {
      try {
        const next = await invoke<AuthView>("get_auth_state");
        if (!disposed) setAuth(next);
        if (!autoResumeAttempted && next.phase === "idle") {
          autoResumeAttempted = true;
          if (await invoke<boolean>("has_saved_login")) {
            await invoke("start_qr_login", {
              contributeCommunitySizes,
            });
          }
        }
      } catch {
        // Browser-only tests do not have the Tauri IPC boundary.
      }
    };
    void refresh();
    const interval = window.setInterval(refresh, 800);
    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, []);

  const startLogin = async () => {
    setCommandError(null);
    setAuth({
      ...initialAuthView,
      phase: "connecting",
      message: "Connecting securely to Steam…",
    });
    try {
      await invoke("start_qr_login", {
        contributeCommunitySizes,
      });
    } catch (error) {
      setCommandError(String(error));
      setAuth(initialAuthView);
    }
  };

  const cancelLogin = async () => {
    setCommandError(null);
    await invoke("cancel_qr_login");
    setAuth(initialAuthView);
  };

  const forgetSavedLogin = async () => {
    setCommandError(null);
    await invoke("forget_saved_login");
    setAuth(initialAuthView);
  };

  if (auth.phase === "complete") {
    return <LibraryApp auth={auth} onSignOut={forgetSavedLogin} />;
  }

  return (
    <Onboarding
      auth={auth}
      commandError={commandError}
      contributeCommunitySizes={contributeCommunitySizes}
      onStart={startLogin}
      onCancel={cancelLogin}
      onContributionChange={setContributeCommunitySizes}
    />
  );
}

function Onboarding({
  auth,
  commandError,
  contributeCommunitySizes,
  onStart,
  onCancel,
  onContributionChange,
}: {
  auth: AuthView;
  commandError: string | null;
  contributeCommunitySizes: boolean;
  onStart: () => Promise<void>;
  onCancel: () => Promise<void>;
  onContributionChange: (enabled: boolean) => void;
}) {
  const busy = !["idle", "error"].includes(auth.phase);
  return (
    <main className="onboarding-shell">
      <Brand compact />
      <section className="onboarding-card" aria-live="polite">
        <div className="onboarding-copy">
          <span className="kicker">Steam library</span>
          <h1>Compare games by size and playtime.</h1>
          <p>
            See storage estimates and lifetime playtime for every game in your
            library.
          </p>
        </div>
        <div className="connect-panel">
          {auth.qrImage ? (
            <>
              <div className="qr-frame">
                <img src={auth.qrImage} alt="Steam sign-in QR code" />
              </div>
              <h2>Approve in Steam Mobile</h2>
              <p>Open Steam Guard, scan this code, then approve the device.</p>
            </>
          ) : busy ? (
            <>
              <span className="large-loader" aria-hidden="true" />
              <h2>Building your library</h2>
              <p>{auth.message}</p>
              <LoadingSteps phase={auth.phase} />
            </>
          ) : (
            <>
              <div className="steam-orbit" aria-hidden="true">
                <span />
              </div>
              <h2>Connect your Steam library</h2>
              <p>{auth.error ?? commandError ?? auth.message}</p>
              <button className="primary-button" onClick={onStart}>
                {auth.phase === "error" ? "Try again" : "Connect with Steam"}
                <span aria-hidden="true">→</span>
              </button>
              <div className="contribution-option">
                <label>
                  <input
                    type="checkbox"
                    role="switch"
                    checked={contributeCommunitySizes}
                    onChange={(event) =>
                      onContributionChange(event.currentTarget.checked)
                    }
                  />
                  <span aria-hidden="true" />
                  Contribute installed sizes
                </label>
                <button
                  type="button"
                  className="tooltip-button"
                  aria-label="About community contributions"
                  data-tooltip="The community database helps estimate the size of games that are not installed. Only public game details and the observed install size are sent—never your Steam ID, profile, playtime, or other personal data."
                >
                  ?
                </button>
              </div>
            </>
          )}
          {busy && (
            <button className="quiet-button" onClick={onCancel}>
              Cancel
            </button>
          )}
        </div>
      </section>
      <p className="onboarding-footnote">
        Steam Storage Optimiser is not affiliated with Valve Corporation.
      </p>
    </main>
  );
}

function LoadingSteps({ phase }: { phase: string }) {
  const active =
    phase === "probing_depots" ? 2 : phase === "fetching_library" ? 1 : 0;
  return (
    <div className="loading-steps">
      {["Authenticate", "Read library", "Measure storage"].map((label, index) => (
        <span
          className={index < active ? "done" : index === active ? "active" : ""}
          key={label}
        >
          <i>{index < active ? "✓" : index + 1}</i>
          {label}
        </span>
      ))}
    </div>
  );
}

function LibraryApp({
  auth,
  onSignOut,
}: {
  auth: AuthView;
  onSignOut: () => Promise<void>;
}) {
  const [query, setQuery] = useState("");
  const [sourceMode, setSourceMode] = useState<SourceMode>("depot");
  const [scope, setScope] = useState<LibraryScope>("all");
  const [sort, setSort] = useState<SortMode>("efficiency");
  const [hideShared, setHideShared] = useState(false);
  const [hideIncompatible, setHideIncompatible] = useState(false);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [storageTarget, setStorageTarget] = useState(() => {
    try {
      const stored = Number(
        window.localStorage.getItem(storageTargetPreferenceKey),
      );
      if (Number.isFinite(stored) && stored > 0) {
        return { bytes: stored, customized: true };
      }
    } catch {
      // A restricted webview can still use the in-memory fallback.
    }
    return { bytes: tib, customized: false };
  });
  const [steamFilesystemSize, setSteamFilesystemSize] = useState<number | null>(
    null,
  );

  useEffect(() => {
    let disposed = false;
    void invoke<StorageTargetDefault>("get_storage_target_default")
      .then((defaults) => {
        if (
          disposed ||
          !defaults ||
          !Number.isFinite(defaults.targetBytes) ||
          !Number.isFinite(defaults.filesystemSizeBytes)
        ) {
          return;
        }
        setSteamFilesystemSize(defaults.filesystemSizeBytes);
        setStorageTarget((current) =>
          current.customized
            ? current
            : { bytes: defaults.targetBytes, customized: false },
        );
      })
      .catch(() => {
        // The 1 TiB fallback remains usable if filesystem detection fails.
      });
    return () => {
      disposed = true;
    };
  }, []);

  const visibleGames = useMemo(
    () =>
      filterAndSortGames(auth.games, {
        query,
        scope,
        hideShared,
        hideIncompatible,
        sort,
        sourceMode,
      }),
    [
      auth.games,
      hideIncompatible,
      hideShared,
      query,
      scope,
      sort,
      sourceMode,
    ],
  );
  const cumulativeSizes = useMemo(
    () => buildCumulativeSizes(visibleGames, sourceMode),
    [sourceMode, visibleGames],
  );
  const maximumStorageSize = getStorageScaleMaximum(
    visibleGames.map(
      (game) => getGameMetrics(game, sourceMode)?.upperSizeBytes ?? 0,
    ),
  );
  const maximumCumulativeSize =
    cumulativeSizes[cumulativeSizes.length - 1]?.lowerSizeBytes ?? 0;
  const storageTargetMaximum = Math.max(
    tib,
    Math.ceil(
      Math.max(maximumCumulativeSize, steamFilesystemSize ?? 0) /
        (100 * gib),
    ) *
      100 *
      gib,
  );
  const storageTargetFill =
    ((storageTarget.bytes - storageTargetMinimum) /
      (storageTargetMaximum - storageTargetMinimum)) *
    100;
  const selectedGame =
    auth.games.find((game) => game.appId === selectedId) ?? null;
  const installedGames = auth.games.filter((game) => game.installed);
  const sharedGames = auth.games.filter((game) => game.sharedOnly);
  const diskUsed = installedGames.reduce(
    (total, game) => total + (game.localSizeBytes ?? 0),
    0,
  );
  const measurable = auth.games.filter(
    (game) => getGameMetrics(game, sourceMode)?.lowerHoursPerGiB !== null,
  );
  const averageEfficiency =
    measurable.reduce(
      (total, game) =>
        total + (getGameMetrics(game, sourceMode)?.lowerHoursPerGiB ?? 0),
      0,
    ) / Math.max(measurable.length, 1);
  const updateStorageTarget = (bytes: number) => {
    setStorageTarget({ bytes, customized: true });
    try {
      window.localStorage.setItem(storageTargetPreferenceKey, String(bytes));
    } catch {
      // The selected target still applies for this session.
    }
  };

  return (
    <main className="product-shell">
      <aside className="sidebar">
        <Brand />
        <nav aria-label="Main navigation">
          <button className="nav-item active">
            <span aria-hidden="true">▦</span> Library
            <small>{auth.games.length}</small>
          </button>
        </nav>
        <div className="account-panel">
          <div className="account-identity">
            <span className="avatar">
              {auth.profile?.displayName.slice(0, 1).toUpperCase() ?? "S"}
              {auth.profile?.avatarUrl && (
                <img src={auth.profile.avatarUrl} alt="" referrerPolicy="no-referrer" />
              )}
            </span>
            <span className="account-copy">
              <strong>{auth.profile?.displayName ?? "Steam account"}</strong>
            </span>
          </div>
          <button
            className="disconnect-button"
            aria-label="Disconnect Steam account"
            onClick={onSignOut}
          >
            <span aria-hidden="true">↪</span>
            <span>Disconnect</span>
          </button>
        </div>
      </aside>

      <section className="main-pane">
        <header className="app-header">
          <div>
            <p className="kicker">Storage overview</p>
            <h1>Your library</h1>
          </div>
          <label className="global-search">
            <span aria-hidden="true">⌕</span>
            <input
              aria-label="Search library"
              placeholder="Search your games"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            <kbd>⌘ K</kbd>
          </label>
          <span className="sync-state">
            <i />
            Synced just now
          </span>
        </header>

        <section className="summary-grid" aria-label="Library summary">
          <SummaryCard
            accent="blue"
            label="Total games"
            value={auth.games.length.toLocaleString()}
            detail={`${sharedGames.length} shared through Steam Family`}
            icon="▦"
          />
          <SummaryCard
            accent="green"
            label="Installed"
            value={installedGames.length.toLocaleString()}
            detail={`${formatBytes(diskUsed)} on disk`}
            icon="↓"
          />
          <SummaryCard
            accent="violet"
            label="Average value"
            value={`${averageEfficiency.toFixed(1)} h/GB`}
            detail={`${measurable.length} games with size data`}
            icon="↗"
          />
        </section>

        {auth.communityError && (
          <div className="notice-banner">
            <span>!</span>
            Community estimates are temporarily unavailable. Depot and local
            sizes are unaffected.
          </div>
        )}
        {auth.depotError && (
          <div className="notice-banner">
            <span>!</span>
            Steam&apos;s depot service is temporarily unavailable. Local and
            community sizes are unaffected.
          </div>
        )}
        {auth.depotProgress &&
          auth.depotProgress.completed < auth.depotProgress.total && (
            <div className="depot-progress" aria-live="polite">
              <span>
                Measuring Steam depots{" "}
                <strong>
                  {auth.depotProgress.completed} / {auth.depotProgress.total}
                </strong>
              </span>
              <i>
                <b
                  style={{
                    width: `${(auth.depotProgress.completed / Math.max(auth.depotProgress.total, 1)) * 100}%`,
                  }}
                />
              </i>
              <small>
                You can use the library while estimates arrive in the
                background.
              </small>
            </div>
          )}
        {auth.depotProgress &&
          auth.depotProgress.completed === auth.depotProgress.total &&
          auth.depotProgress.available > 0 &&
          auth.depotProgress.unavailable > 0 && (
            <div className="coverage-note">
              Steam depot estimates cover{" "}
              <strong>
                {auth.depotProgress.available} of {auth.depotProgress.total}
              </strong>{" "}
              games. Missing estimates use the selected fallback where
              available.
            </div>
          )}

        <section className="library-panel">
          <div className="storage-target-control">
            <div>
              <label htmlFor="storage-target">
                Library size target
                <strong>{formatStorageTarget(storageTarget.bytes)}</strong>
              </label>
              <small>
                {storageTarget.customized
                  ? "Saved on this device"
                  : steamFilesystemSize
                    ? `Suggested from your ${formatBytes(steamFilesystemSize)} primary Steam drive`
                    : "Using the 1 TiB fallback"}
              </small>
            </div>
            <input
              aria-label="Library size target"
              id="storage-target"
              type="range"
              min={storageTargetMinimum}
              max={storageTargetMaximum}
              step={10 * gib}
              value={storageTarget.bytes}
              style={
                {
                  "--target-fill": `${Math.min(Math.max(storageTargetFill, 0), 100)}%`,
                } as CSSProperties
              }
              onChange={(event) =>
                updateStorageTarget(Number(event.target.value))
              }
            />
            <span className="target-scale">
              <small>10 GiB</small>
              <small>{formatStorageTarget(storageTargetMaximum)}</small>
            </span>
          </div>
          <div className="library-toolbar">
            <div className="scope-tabs" aria-label="Library scope">
              {(["all", "installed", "uninstalled"] as LibraryScope[]).map(
                (value) => (
                  <button
                    className={scope === value ? "active" : ""}
                    key={value}
                    onClick={() => setScope(value)}
                  >
                    {value[0].toUpperCase() + value.slice(1)}
                  </button>
                ),
              )}
            </div>
            <div className="toolbar-actions">
              <label className="shared-toggle">
                <input
                  type="checkbox"
                  checked={hideShared}
                  onChange={(event) => setHideShared(event.target.checked)}
                />
                <span />
                Hide shared
              </label>
              <label className="shared-toggle">
                <input
                  type="checkbox"
                  checked={hideIncompatible}
                  onChange={(event) =>
                    setHideIncompatible(event.target.checked)
                  }
                />
                <span />
                Hide incompatible
              </label>
              <label className="select-control">
                <span>Size source</span>
                <select
                  aria-label="Size source"
                  value={sourceMode}
                  onChange={(event) =>
                    setSourceMode(event.target.value as SourceMode)
                  }
                >
                  <option value="compare">Compare</option>
                  <option value="depot">Steam depot</option>
                  <option value="community">Community</option>
                </select>
              </label>
              <label className="select-control">
                <span>Sort</span>
                <select
                  aria-label="Sort library"
                  value={sort}
                  onChange={(event) =>
                    setSort(event.target.value as SortMode)
                  }
                >
                  <option value="efficiency">Hours per GB</option>
                  <option value="playtime">Playtime</option>
                  <option value="size">Size</option>
                  <option value="name">Name</option>
                </select>
              </label>
            </div>
          </div>

          <div className="table-caption">
            <span>
              Showing <strong>{visibleGames.length}</strong> games
            </span>
            <p>
              Installed games always use Steam&apos;s exact local size.
              Uninstalled games use your selected estimate source.
            </p>
          </div>

          <div className="game-table" role="table" aria-label="Steam library">
            <div className="game-row table-head" role="row">
              <span role="columnheader">Game</span>
              <span role="columnheader">Playtime</span>
              <span role="columnheader">Storage</span>
              <span role="columnheader">Cumulative</span>
              <span role="columnheader">Hours / GB</span>
              <span role="columnheader">Source</span>
              <span />
            </div>
            {visibleGames.map((game, index) => (
              <GameRow
                cumulative={cumulativeSizes[index]}
                game={game}
                key={game.appId}
                maximumCumulativeSize={maximumCumulativeSize}
                maximumStorageSize={maximumStorageSize}
                mode={sourceMode}
                onCompare={() => setSourceMode("compare")}
                onSelect={() => setSelectedId(game.appId)}
                storageTargetSize={storageTarget.bytes}
              />
            ))}
            {visibleGames.length === 0 && (
              <div className="empty-library">
                <span>⌕</span>
                <h2>No games match these filters</h2>
                <p>Try another search or include shared games.</p>
              </div>
            )}
          </div>
        </section>
      </section>

      {selectedGame && (
        <GameDrawer
          game={selectedGame}
          mode={sourceMode}
          probe={auth.probe?.appId === selectedGame.appId ? auth.probe : null}
          onClose={() => setSelectedId(null)}
        />
      )}
    </main>
  );
}

function Brand({ compact = false }: { compact?: boolean }) {
  return (
    <div className={`brand ${compact ? "compact" : ""}`}>
      <svg
        className="brand-mark"
        viewBox="0 0 36 36"
        aria-hidden="true"
      >
        <defs>
          <linearGradient id="storage-mark-gradient" x1="5" y1="31" x2="31" y2="5">
            <stop stopColor="#1a6fa1" />
            <stop offset="1" stopColor="#66c0f4" />
          </linearGradient>
        </defs>
        <rect width="36" height="36" rx="10" fill="url(#storage-mark-gradient)" />
        {[8, 15.25, 22.5].map((y) => (
          <g key={y}>
            <rect
              x="7.5"
              y={y}
              width="21"
              height="5.5"
              rx="2"
              fill="rgba(7, 24, 36, 0.38)"
              stroke="rgba(255, 255, 255, 0.9)"
              strokeWidth="1.25"
            />
            <circle cx="24.75" cy={y + 2.75} r="1" fill="#fff" />
          </g>
        ))}
      </svg>
      <div>
        <strong>Storage Optimiser</strong>
        <small>for Steam</small>
      </div>
    </div>
  );
}

function SummaryCard({
  accent,
  label,
  value,
  detail,
  icon,
}: {
  accent: string;
  label: string;
  value: string;
  detail: string;
  icon: string;
}) {
  return (
    <article className={`summary-card accent-${accent}`}>
      <span className="summary-icon">{icon}</span>
      <div>
        <p>{label}</p>
        <strong>{value}</strong>
        <small>{detail}</small>
      </div>
    </article>
  );
}

function GameRow({
  cumulative,
  game,
  maximumCumulativeSize,
  maximumStorageSize,
  mode,
  onCompare,
  onSelect,
  storageTargetSize,
}: {
  cumulative: CumulativeSize;
  game: LibraryGame;
  maximumCumulativeSize: number;
  maximumStorageSize: number;
  mode: SourceMode;
  onCompare: () => void;
  onSelect: () => void;
  storageTargetSize: number;
}) {
  const metrics = getGameMetrics(game, mode);
  const efficiencyBar = getEfficiencyBarWidths(metrics);
  const storageBar = getStorageBarWidths(metrics, maximumStorageSize);
  const addedCumulativeSize = metrics?.lowerSizeBytes ?? 0;
  const cumulativeBar = getCumulativeBarWidths(
    cumulative,
    addedCumulativeSize,
    storageTargetSize,
    maximumCumulativeSize,
  );
  const showDiscrepancy =
    !game.installed &&
    mode !== "compare" &&
    hasLargeSizeDiscrepancy(game);
  return (
    <div
      className="game-row interactive-row"
      role="row"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (
          event.target === event.currentTarget &&
          (event.key === "Enter" || event.key === " ")
        ) {
          event.preventDefault();
          onSelect();
        }
      }}
    >
      <span className="game-identity" role="cell">
        <GameArtwork game={game} />
        <span>
          <span className="game-title-line">
            <strong>{game.name}</strong>
            {game.currentOsSupported === false && <WindowsOnly />}
          </span>
          <small>
            <span className="app-id">App {game.appId}</span>
            {game.installed && <em className="installed-tag">Installed</em>}
            {game.sharedOnly && <em className="shared-tag">Shared</em>}
          </small>
        </span>
      </span>
      <span className="numeric-cell" role="cell">
        <strong>{formatPlaytime(game.playtimeMinutes)}</strong>
        <small>lifetime</small>
      </span>
      <span className="numeric-cell" role="cell">
        <span className="storage-value-line">
          <strong>{formatSizeMetric(metrics)}</strong>
          {showDiscrepancy && (
            <span
              className="discrepancy-warning"
              onClick={(event) => event.stopPropagation()}
            >
              <button
                aria-label={`Size estimates differ for ${game.name}`}
                className="discrepancy-trigger"
                type="button"
              >
                <svg aria-hidden="true" viewBox="0 0 16 16">
                  <path d="M8 1.5 15 14H1L8 1.5Z" />
                  <path d="M8 5v4.5M8 12v.2" />
                </svg>
              </button>
              <span className="discrepancy-popover" role="tooltip">
                <strong>Size estimates differ</strong>
                <p>
                  <b>Depot</b>
                  May exclude launchers, bootstrapping, or compatibility files.
                </p>
                <p>
                  <b>Community</b>
                  May be out of date or based on a different operating system.
                </p>
                <span>
                  Community size
                  <b>{formatBytes(game.communitySizeBytes ?? 0)}</b>
                </span>
                <button
                  type="button"
                  onClick={(event) => {
                    event.stopPropagation();
                    onCompare();
                  }}
                >
                  Compare sources
                </button>
              </span>
            </span>
          )}
        </span>
        {storageBar && (
          <i
            className={`metric-bar storage-bar${storageBar.capped ? " capped" : ""}`}
            aria-label={
              storageBar.rangePercent > 0 ? "Storage range" : "Storage"
            }
          >
            <span
              className="storage-base"
              style={{ width: `${storageBar.basePercent}%` }}
            />
            {storageBar.rangePercent > 0 && (
              <span
                className="storage-range"
                style={{
                  left: `${storageBar.basePercent}%`,
                  width: `${storageBar.rangePercent}%`,
                }}
              />
            )}
          </i>
        )}
        <small>{game.installed ? "on disk" : "estimated"}</small>
      </span>
      <span className="numeric-cell cumulative-cell" role="cell">
        <strong>{formatCumulativeSize(cumulative)}</strong>
        {cumulativeBar && (
          <i
            className={`metric-bar cumulative-bar${cumulativeBar.overTarget ? " over-target" : ""}`}
            aria-label={
              cumulativeBar.overTarget
                ? "Cumulative storage over target"
                : "Cumulative storage"
            }
          >
            <span
              className="cumulative-previous"
              style={{ width: `${cumulativeBar.previousPercent}%` }}
            />
            {cumulativeBar.addedPercent > 0 && (
              <span
                className="cumulative-added"
                style={{
                  left: `${cumulativeBar.previousPercent}%`,
                  width: `${cumulativeBar.addedPercent}%`,
                }}
              />
            )}
          </i>
        )}
        <small>
          {cumulative.unknownCount > 0
            ? `${cumulative.unknownCount} unknown`
            : "visible total"}
        </small>
      </span>
      <span className="efficiency-cell" role="cell">
        <strong>{formatEfficiency(metrics)}</strong>
        {efficiencyBar && (
          <i
            className="metric-bar efficiency-bar"
            aria-label={
              efficiencyBar.rangePercent > 0
                ? "Hours per GB range"
                : "Hours per GB"
            }
          >
            <span
              className="efficiency-base"
              style={{
                width: `${efficiencyBar.basePercent}%`,
              }}
            />
            {efficiencyBar.rangePercent > 0 && (
              <span
                className="efficiency-range"
                style={{
                  left: `${efficiencyBar.basePercent}%`,
                  width: `${efficiencyBar.rangePercent}%`,
                }}
              />
            )}
          </i>
        )}
      </span>
      <span className="source-cell" role="cell">
        {metrics ? (
          metrics.sources.map((source) => (
            <em className={`source-badge source-${source}`} key={source}>
              {source}
            </em>
          ))
        ) : (
          <em className="source-badge source-pending">Pending</em>
        )}
        {metrics?.fallback && <small>fallback</small>}
        {metrics?.collapsedComparison && (
          <small className="agreement-label">close match</small>
        )}
      </span>
      <span className="row-arrow" aria-hidden="true">
        ›
      </span>
    </div>
  );
}

function GameDrawer({
  game,
  mode,
  probe,
  onClose,
}: {
  game: LibraryGame;
  mode: SourceMode;
  probe: DepotProbe | null;
  onClose: () => void;
}) {
  const metrics = getGameMetrics(game, mode);
  return (
    <div className="drawer-backdrop" onMouseDown={onClose}>
      <aside
        className="game-drawer"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <button className="drawer-close" aria-label="Close details" onClick={onClose}>
          ×
        </button>
        <GameArtwork game={game} large />
        <p className="kicker">App {game.appId}</p>
        <div className="drawer-title-line">
          <h2>{game.name}</h2>
          {game.currentOsSupported === false && <WindowsOnly />}
        </div>
        <div className="drawer-tags">
          {game.installed && <em className="installed-tag">Installed</em>}
          {game.sharedOnly && <em className="shared-tag">Steam Family</em>}
        </div>
        <div className="drawer-metrics">
          <article>
            <small>Lifetime playtime</small>
            <strong>{formatPlaytime(game.playtimeMinutes)}</strong>
          </article>
          <article>
            <small>Storage</small>
            <strong>{formatSizeMetric(metrics)}</strong>
          </article>
          <article>
            <small>Value</small>
            <strong>{formatEfficiency(metrics)}</strong>
          </article>
        </div>
        <section className="source-breakdown">
          <h3>Size sources</h3>
          <SourceLine
            label="Local installation"
            value={game.localSizeBytes}
            state={game.installed ? "Exact" : "Not installed"}
          />
          <SourceLine
            label="Steam depots"
            value={game.depotSizeBytes}
            state={
              game.depotStatus === "pending"
                ? "Measuring"
                : game.depotStatus === "unavailable"
                  ? "Unavailable"
                  : game.depotExact
                    ? "File-level manifest"
                    : `${game.depotOs === "windows" ? "Windows · " : ""}${game.depotCount ?? 0} depot manifest summary`
            }
          />
          <SourceLine
            label="Community"
            value={game.communitySizeBytes}
            state={game.communitySizeBytes ? "Historical estimate" : "No observation"}
          />
        </section>
        {game.sharedOnly && (
          <div className="drawer-note">
            <strong>Shared through Steam Family</strong>
            <p>
              Installation estimates follow Steam&apos;s preferred family
              owner and that owner&apos;s available DLC.
            </p>
          </div>
        )}
        {game.depotWarnings.length > 0 && (
          <div className="drawer-note">
            <strong>Depot notes</strong>
            <p>{game.depotWarnings.join(" · ")}</p>
          </div>
        )}
        {game.depotStatus === "available" &&
          !game.depotExact &&
          (game.depotCount ?? 0) > 1 && (
            <div className="drawer-note">
              <strong>Fast manifest estimate</strong>
              <p>
                This adds Steam&apos;s uncompressed summary for each selected
                depot. If depots write to the same path, a future file-level
                refinement may reduce the total.
              </p>
            </div>
          )}
        {game.currentOsSupported === false && (
          <div className="drawer-note platform-note">
            <strong>Not available for this operating system</strong>
            <p>
              This storage figure uses the Windows depot selection instead.
              Platform-neutral DLC alone is not treated as a compatible base
              game.
            </p>
          </div>
        )}
        {game.depotError && (
          <div className="drawer-note warning-note">
            <strong>Depot estimate unavailable</strong>
            <p>{game.depotError}</p>
          </div>
        )}
        {probe && (
          <section className="depot-details">
            <h3>Depot calculation</h3>
            <p>
              {probe.depots.length} public-branch depots, merged by destination
              path. {probe.selectionWarnings.length || "No"} warnings.
            </p>
          </section>
        )}
      </aside>
    </div>
  );
}

function SourceLine({
  label,
  value,
  state,
}: {
  label: string;
  value: number | null;
  state: string;
}) {
  return (
    <div>
      <span>
        <strong>{label}</strong>
        <small>{state}</small>
      </span>
      <b>{value ? formatBytes(value) : "—"}</b>
    </div>
  );
}

function WindowsIcon() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M1 2.35 7.15 1.5v5.9H1V2.35Zm6.85-.95L15 0.4v7H7.85v-6Zm-6.85 6.7h6.15V14L1 13.15V8.1Zm6.85 0H15v7l-7.15-1V8.1Z" />
    </svg>
  );
}

function WindowsOnly() {
  return (
    <span
      className="windows-only"
      title="Not available on this operating system; using the Windows depot estimate"
    >
      <WindowsIcon />
      only
    </span>
  );
}

function formatPlaytime(minutes: number | null) {
  if (minutes === null) return "Unknown";
  if (minutes < 60) return `${minutes}m`;
  return `${Math.round(minutes / 60).toLocaleString()}h`;
}

export function gameArtworkUrl(appId: number) {
  return `https://cdn.cloudflare.steamstatic.com/steam/apps/${appId}/header.jpg`;
}

function GameArtwork({
  game,
  large = false,
}: {
  game: LibraryGame;
  large?: boolean;
}) {
  return (
    <i
      className={`${large ? "drawer-hero" : "game-art"} art-${game.appId % 6}`}
      aria-hidden="true"
    >
      <span>{game.name.slice(0, 2).toUpperCase()}</span>
      <img
        src={gameArtworkUrl(game.appId)}
        alt=""
        loading="lazy"
        referrerPolicy="no-referrer"
        onError={(event) => event.currentTarget.remove()}
      />
    </i>
  );
}

function formatSizeMetric(metrics: GameMetrics) {
  if (!metrics) return "Unknown";
  if (metrics.lowerSizeBytes === metrics.upperSizeBytes) {
    return formatBytes(metrics.lowerSizeBytes);
  }
  return formatByteRange(metrics.lowerSizeBytes, metrics.upperSizeBytes);
}

function formatCumulativeSize(cumulative: CumulativeSize) {
  if (
    cumulative.unknownCount > 0 &&
    cumulative.lowerSizeBytes === 0 &&
    cumulative.upperSizeBytes === 0
  ) {
    return "Unknown";
  }
  if (cumulative.unknownCount > 0) {
    return `≥ ${formatBytes(cumulative.lowerSizeBytes)}`;
  }
  if (cumulative.lowerSizeBytes === cumulative.upperSizeBytes) {
    return formatBytes(cumulative.lowerSizeBytes);
  }
  return formatByteRange(
    cumulative.lowerSizeBytes,
    cumulative.upperSizeBytes,
  );
}

function formatEfficiency(metrics: GameMetrics) {
  if (!metrics || metrics.lowerHoursPerGiB === null) return "—";
  if (
    metrics.upperHoursPerGiB !== null &&
    Math.abs(metrics.upperHoursPerGiB - metrics.lowerHoursPerGiB) > 0.05
  ) {
    return `${metrics.lowerHoursPerGiB.toFixed(1)} → ${metrics.upperHoursPerGiB.toFixed(1)} h/GB`;
  }
  return `${metrics.lowerHoursPerGiB.toFixed(1)} h/GB`;
}

function storageUnit(bytes: number) {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const exponent =
    bytes === 0
      ? 0
      : Math.min(
          Math.floor(Math.log(bytes) / Math.log(1024)),
          units.length - 1,
        );
  return {
    divisor: 1024 ** exponent,
    exponent,
    unit: units[exponent],
  };
}

export function formatByteRange(lower: number, upper: number) {
  const lowerUnit = storageUnit(lower);
  const upperUnit = storageUnit(upper);
  if (lowerUnit.exponent === upperUnit.exponent) {
    const decimals = upperUnit.exponent > 1 ? 2 : 0;
    return `${(lower / lowerUnit.divisor).toFixed(decimals)} → ${(upper / upperUnit.divisor).toFixed(decimals)} ${upperUnit.unit}`;
  }
  return `${formatBytes(lower)} → ${formatBytes(upper)}`;
}

export default App;
