import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { type CSSProperties, useEffect, useMemo, useState } from "react";
import "./App.css";

let autoResumeAttempted = false;
const contributionPreferenceKey = "contribute-community-sizes";
const storageTargetPreferenceKey = "storage-target-bytes";
const completionModePreferenceKey = "hltb-completion-mode";

export type LibraryGame = {
  appId: number;
  name: string;
  appType: string;
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
  hltb: HltbEstimate | null;
  hltbStatus: "pending" | "matched" | "unmatched" | "not_applicable";
};

export type HltbEstimate = {
  gameId: number;
  gameName: string;
  mainSeconds: number | null;
  mainExtraSeconds: number | null;
  completionistSeconds: number | null;
  steamAppId: number | null;
  matchMethod: "steam_app_id" | "title" | "manual";
};

export type HltbCandidate = Omit<HltbEstimate, "matchMethod"> & {
  platforms: string;
  similarity: number;
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
  hltbError: string | null;
  hltbProgress: HltbProgress | null;
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

export type SteamLocation = {
  path: string | null;
  source: "saved" | "automatic" | null;
};

export type DepotProgress = {
  completed: number;
  total: number;
  available: number;
  unavailable: number;
};

export type HltbProgress = {
  completed: number;
  total: number;
  matched: number;
  unmatched: number;
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
  hltbError: null,
  hltbProgress: null,
  profile: null,
};

export type SourceMode = "depot" | "community" | "compare";
export type LibraryScope = "all" | "installed" | "uninstalled";
export type SortMode =
  | "efficiency"
  | "remaining"
  | "remaining_hours"
  | "playtime"
  | "size"
  | "name";
export type CompletionMode = "main" | "main_extra" | "completionist";
export type SortDirection = "ascending" | "descending";

export type EfficiencyMetric = {
  lowerHoursPerGiB: number | null;
  upperHoursPerGiB: number | null;
} | null;

export type GameMetrics = {
  lowerSizeBytes: number;
  upperSizeBytes: number;
  lowerHoursPerGiB: number | null;
  upperHoursPerGiB: number | null;
  sources: Array<"local" | "depot" | "community">;
  fallback: boolean;
  collapsedComparison: boolean;
} | null;

export type RemainingMetrics = {
  lowerHoursPerGiB: number;
  upperHoursPerGiB: number;
  remainingHours: number;
  fallbackExplanation: string | null;
} | null;

export type RemainingTime = {
  remainingHours: number;
  fallbackExplanation: string | null;
} | null;

export type HltbTarget = {
  seconds: number;
  fallbackExplanation: string | null;
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

export function getRemainingMetrics(
  game: LibraryGame,
  sourceMode: SourceMode,
  completionMode: CompletionMode,
): RemainingMetrics {
  const size = getGameMetrics(game, sourceMode);
  const remaining = getRemainingTime(game, completionMode);
  if (!size || !remaining) return null;
  return {
    remainingHours: remaining.remainingHours,
    lowerHoursPerGiB:
      remaining.remainingHours / (size.upperSizeBytes / gib),
    upperHoursPerGiB:
      remaining.remainingHours / (size.lowerSizeBytes / gib),
    fallbackExplanation: remaining.fallbackExplanation,
  };
}

export function getRemainingTime(
  game: LibraryGame,
  completionMode: CompletionMode,
): RemainingTime {
  if (game.playtimeMinutes === null || !game.hltb) return null;
  const target = getHltbTarget(game.hltb, completionMode);
  if (!target) return null;
  return {
    remainingHours: Math.max(
      target.seconds / 3600 - game.playtimeMinutes / 60,
      0,
    ),
    fallbackExplanation: target.fallbackExplanation,
  };
}

export function getHltbTarget(
  estimate: HltbEstimate,
  completionMode: CompletionMode,
): HltbTarget {
  const main = estimate.mainSeconds;
  const extras = estimate.mainExtraSeconds;
  const completionist = estimate.completionistSeconds;
  if (completionMode === "main") {
    if (main) return { seconds: main, fallbackExplanation: null };
    if (extras) {
      return {
        seconds: extras,
        fallbackExplanation:
          "Main Story is unavailable, so this uses Main + Extras.",
      };
    }
    if (completionist) {
      return {
        seconds: completionist,
        fallbackExplanation:
          "Main Story and Main + Extras are unavailable, so this uses Completionist.",
      };
    }
    return null;
  }
  if (completionMode === "main_extra") {
    if (extras) return { seconds: extras, fallbackExplanation: null };
    if (main && completionist) {
      return {
        seconds: (main + completionist) / 2,
        fallbackExplanation:
          "Main + Extras is unavailable, so this uses the midpoint between Main Story and Completionist.",
      };
    }
    if (completionist) {
      return {
        seconds: completionist,
        fallbackExplanation:
          "Main + Extras is unavailable, so this uses Completionist.",
      };
    }
    if (main) {
      return {
        seconds: main,
        fallbackExplanation:
          "Main + Extras is unavailable, so this uses Main Story.",
      };
    }
    return null;
  }
  if (completionist) {
    return { seconds: completionist, fallbackExplanation: null };
  }
  if (extras) {
    return {
      seconds: extras,
      fallbackExplanation:
        "Completionist is unavailable, so this uses Main + Extras.",
    };
  }
  if (main) {
    return {
      seconds: main,
      fallbackExplanation:
        "Completionist and Main + Extras are unavailable, so this uses Main Story.",
    };
  }
  return null;
}

export function filterAndSortGames(
  games: LibraryGame[],
  options: {
    query: string;
    scope: LibraryScope;
    hideShared: boolean;
    hideIncompatible: boolean;
    showNonGames?: boolean;
    sort: SortMode;
    sortDirection?: SortDirection;
    sourceMode: SourceMode;
    completionMode?: CompletionMode;
  },
) {
  const query = options.query.trim().toLocaleLowerCase();
  return games
    .filter((game) => {
      if (query && !game.name.toLocaleLowerCase().includes(query)) return false;
      if (options.hideShared && game.sharedOnly) return false;
      if (options.hideIncompatible && game.currentOsSupported === false)
        return false;
      if (
        options.showNonGames === false &&
        game.appType !== "game" &&
        game.appType !== "unknown"
      )
        return false;
      if (options.scope === "installed" && !game.installed) return false;
      if (options.scope === "uninstalled" && game.installed) return false;
      return true;
    })
    .sort((left, right) => {
      const defaultDirection =
        options.sort === "name" ? "ascending" : "descending";
      const applyDirection = (comparison: number) =>
        options.sortDirection &&
        options.sortDirection !== defaultDirection
          ? -comparison
          : comparison;
      const leftMetric = getGameMetrics(left, options.sourceMode);
      const rightMetric = getGameMetrics(right, options.sourceMode);
      if (options.sort === "remaining") {
        const completionMode = options.completionMode ?? "main_extra";
        return applyDirection(
          (getRemainingMetrics(right, options.sourceMode, completionMode)
            ?.lowerHoursPerGiB ?? -1) -
            (getRemainingMetrics(left, options.sourceMode, completionMode)
              ?.lowerHoursPerGiB ?? -1) ||
            left.name.localeCompare(right.name),
        );
      }
      if (options.sort === "remaining_hours") {
        const completionMode = options.completionMode ?? "main_extra";
        return applyDirection(
          (getRemainingTime(right, completionMode)?.remainingHours ?? -1) -
            (getRemainingTime(left, completionMode)?.remainingHours ?? -1) ||
            left.name.localeCompare(right.name),
        );
      }
      if (options.sort === "name") {
        return applyDirection(left.name.localeCompare(right.name));
      }
      if (options.sort === "playtime") {
        return applyDirection(
          (right.playtimeMinutes ?? -1) - (left.playtimeMinutes ?? -1) ||
            left.name.localeCompare(right.name),
        );
      }
      if (options.sort === "size") {
        return applyDirection(
          (rightMetric?.upperSizeBytes ?? -1) -
            (leftMetric?.upperSizeBytes ?? -1) ||
            left.name.localeCompare(right.name),
        );
      }
      return applyDirection(
        (rightMetric?.lowerHoursPerGiB ?? -1) -
          (leftMetric?.lowerHoursPerGiB ?? -1) ||
          left.name.localeCompare(right.name),
      );
    });
}

function isGameApplication(game: LibraryGame) {
  return game.appType === "game" || game.appType === "unknown";
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
  metrics: EfficiencyMetric,
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
  return getAdaptiveBarWidths(
    metrics.lowerSizeBytes,
    metrics.upperSizeBytes,
    maximumSizeBytes,
  );
}

export function getAdaptiveBarWidths(
  lowerValue: number,
  upperValue: number,
  maximumValue: number,
): StorageBarWidths | null {
  if (maximumValue <= 0) return null;
  const basePercent = Math.min(
    Math.max((lowerValue / maximumValue) * 100, 0),
    100,
  );
  const upperPercent = Math.min(
    Math.max((upperValue / maximumValue) * 100, 0),
    100,
  );
  return {
    basePercent,
    rangePercent: Math.max(upperPercent - basePercent, 0),
    capped: upperValue > maximumValue,
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
  const [steamLocation, setSteamLocation] = useState<
    SteamLocation | undefined
  >();
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

  useEffect(() => {
    void invoke<SteamLocation>("get_steam_location")
      .then(setSteamLocation)
      .catch(() => setSteamLocation({ path: null, source: null }));
  }, []);

  const selectSteamLocation = async () => {
    setCommandError(null);
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Locate your Steam folder",
        defaultPath: steamLocation?.path ?? undefined,
      });
      if (typeof selected !== "string") return;
      const location = await invoke<SteamLocation>("set_steam_location", {
        path: selected,
      });
      setSteamLocation(location);
      if (auth.phase === "complete") {
        window.location.reload();
      }
    } catch (error) {
      setCommandError(String(error));
    }
  };

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
    return (
      <LibraryApp
        auth={auth}
        locationError={commandError}
        steamLocation={steamLocation}
        onLocateSteam={selectSteamLocation}
        onSignOut={forgetSavedLogin}
      />
    );
  }

  return (
    <Onboarding
      auth={auth}
      commandError={commandError}
      contributeCommunitySizes={contributeCommunitySizes}
      onStart={startLogin}
      onCancel={cancelLogin}
      onLocateSteam={selectSteamLocation}
      onContributionChange={setContributeCommunitySizes}
      steamLocation={steamLocation}
    />
  );
}

function Onboarding({
  auth,
  commandError,
  contributeCommunitySizes,
  onStart,
  onCancel,
  onLocateSteam,
  onContributionChange,
  steamLocation,
}: {
  auth: AuthView;
  commandError: string | null;
  contributeCommunitySizes: boolean;
  onStart: () => Promise<void>;
  onCancel: () => Promise<void>;
  onLocateSteam: () => Promise<void>;
  onContributionChange: (enabled: boolean) => void;
  steamLocation: SteamLocation | undefined;
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
              {steamLocation && !steamLocation.path && (
                <div className="steam-location-warning">
                  <span>Steam wasn&apos;t found on this computer.</span>
                  <button type="button" onClick={onLocateSteam}>
                    Locate Steam
                  </button>
                </div>
              )}
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
  locationError,
  steamLocation,
  onLocateSteam,
  onSignOut,
}: {
  auth: AuthView;
  locationError: string | null;
  steamLocation: SteamLocation | undefined;
  onLocateSteam: () => Promise<void>;
  onSignOut: () => Promise<void>;
}) {
  const [query, setQuery] = useState("");
  const [sourceMode, setSourceMode] = useState<SourceMode>("depot");
  const [scope, setScope] = useState<LibraryScope>("all");
  const [sort, setSort] = useState<SortMode>("efficiency");
  const [sortDirection, setSortDirection] =
    useState<SortDirection>("descending");
  const [completionMode, setCompletionMode] = useState<CompletionMode>(() => {
    try {
      const saved = window.localStorage.getItem(completionModePreferenceKey);
      if (saved === "main" || saved === "completionist") return saved;
    } catch {
      // Use the default when storage is unavailable.
    }
    return "main_extra";
  });
  const [hideShared, setHideShared] = useState(false);
  const [hideIncompatible, setHideIncompatible] = useState(false);
  const [showNonGames, setShowNonGames] = useState(false);
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

  useEffect(() => {
    try {
      window.localStorage.setItem(
        completionModePreferenceKey,
        completionMode,
      );
    } catch {
      // The selection still applies for this session.
    }
  }, [completionMode]);

  const visibleGames = useMemo(
    () =>
      filterAndSortGames(auth.games, {
        query,
        scope,
        hideShared,
        hideIncompatible,
        showNonGames,
        sort,
        sortDirection,
        sourceMode,
        completionMode,
      }),
    [
      auth.games,
      hideIncompatible,
      hideShared,
      showNonGames,
      query,
      scope,
      sort,
      sortDirection,
      sourceMode,
      completionMode,
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
  const maximumRemainingHours = getStorageScaleMaximum(
    visibleGames.map(
      (game) =>
        getRemainingTime(game, completionMode)?.remainingHours ?? 0,
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
  const includedLibraryGames = auth.games.filter(
    (game) => showNonGames || isGameApplication(game),
  );
  const installedGames = includedLibraryGames.filter((game) => game.installed);
  const sharedGames = includedLibraryGames.filter((game) => game.sharedOnly);
  const diskUsed = installedGames.reduce(
    (total, game) => total + (game.localSizeBytes ?? 0),
    0,
  );
  const measurable = includedLibraryGames.filter(
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

  const updateSort = (nextSort: SortMode) => {
    if (sort === nextSort) {
      setSortDirection((current) =>
        current === "ascending" ? "descending" : "ascending",
      );
      return;
    }
    setSort(nextSort);
    setSortDirection(nextSort === "name" ? "ascending" : "descending");
  };

  return (
    <main className="product-shell">
      <aside className="sidebar">
        <Brand />
        <nav aria-label="Main navigation">
          <button className="nav-item active">
            <span aria-hidden="true">▦</span> Library
            <small>{includedLibraryGames.length}</small>
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
          <button
            className="steam-folder-button"
            title={steamLocation?.path ?? "Steam folder not found"}
            type="button"
            onClick={onLocateSteam}
          >
            <span aria-hidden="true">⌑</span>
            <span>{steamLocation?.path ? "Change Steam folder" : "Locate Steam"}</span>
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
            label={showNonGames ? "Total items" : "Total games"}
            value={includedLibraryGames.length.toLocaleString()}
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
        {locationError && (
          <div className="notice-banner">
            <span>!</span>
            {locationError}
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
        {auth.hltbError && (
          <div className="notice-banner">
            <span>!</span>
            {auth.hltbError}
          </div>
        )}
        {auth.hltbProgress &&
          auth.hltbProgress.completed < auth.hltbProgress.total && (
            <div className="depot-progress hltb-progress" aria-live="polite">
              <span>
                Matching HowLongToBeat data{" "}
                <strong>
                  {auth.hltbProgress.completed} / {auth.hltbProgress.total}
                </strong>
              </span>
              <i>
                <b
                  style={{
                    width: `${(auth.hltbProgress.completed / Math.max(auth.hltbProgress.total, 1)) * 100}%`,
                  }}
                />
              </i>
              <small>
                Results are cached for six months and appear as they arrive.
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
          <div className="completion-control">
            <div>
              <span>Remaining-time target</span>
              <strong>
                {completionMode === "main"
                  ? "Main Story"
                  : completionMode === "completionist"
                    ? "Completionist"
                    : "Main + Extras"}
              </strong>
            </div>
            <input
              aria-label="HowLongToBeat completion level"
              type="range"
              min="0"
              max="2"
              step="1"
              value={
                completionMode === "main"
                  ? 0
                  : completionMode === "main_extra"
                    ? 1
                    : 2
              }
              onChange={(event) =>
                setCompletionMode(
                  event.target.value === "0"
                    ? "main"
                    : event.target.value === "2"
                      ? "completionist"
                      : "main_extra",
                )
              }
            />
            <span className="completion-labels" aria-hidden="true">
              <small>Main Story</small>
              <small>Main + Extras</small>
              <small>Completionist</small>
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
              <label className="shared-toggle">
                <input
                  type="checkbox"
                  checked={showNonGames}
                  onChange={(event) => setShowNonGames(event.target.checked)}
                />
                <span />
                Show software &amp; tools
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
            </div>
          </div>

          <div className="table-caption">
            <span>
              Showing <strong>{visibleGames.length}</strong>{" "}
              {showNonGames ? "items" : "games"}
            </span>
            <p>
              Installed games always use Steam&apos;s exact local size.
              Uninstalled games use your selected estimate source.
            </p>
          </div>

          <div className="game-table" role="table" aria-label="Steam library">
            <div className="game-row table-head" role="row">
              <SortableHeader
                active={sort === "name"}
                direction={sortDirection}
                label="Game"
                onClick={() => updateSort("name")}
              />
              <SortableHeader
                active={sort === "playtime"}
                direction={sortDirection}
                label="Playtime"
                onClick={() => updateSort("playtime")}
              />
              <SortableHeader
                active={sort === "remaining_hours"}
                direction={sortDirection}
                label="Remaining"
                onClick={() => updateSort("remaining_hours")}
              />
              <SortableHeader
                active={sort === "size"}
                direction={sortDirection}
                label="Storage"
                onClick={() => updateSort("size")}
              />
              <span role="columnheader">Cumulative</span>
              <SortableHeader
                active={sort === "efficiency"}
                direction={sortDirection}
                label="Hours / GB"
                onClick={() => updateSort("efficiency")}
              />
              <SortableHeader
                active={sort === "remaining"}
                direction={sortDirection}
                label="Remaining / GB"
                onClick={() => updateSort("remaining")}
              />
              <span role="columnheader">Source</span>
              <span />
            </div>
            {visibleGames.map((game, index) => (
              <GameRow
                cumulative={cumulativeSizes[index]}
                completionMode={completionMode}
                game={game}
                key={game.appId}
                maximumCumulativeSize={maximumCumulativeSize}
                maximumRemainingHours={maximumRemainingHours}
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
          completionMode={completionMode}
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

function SortableHeader({
  active,
  direction,
  label,
  onClick,
}: {
  active: boolean;
  direction: SortDirection;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      className={`sortable-header${active ? " active" : ""}`}
      role="columnheader"
      aria-sort={active ? direction : "none"}
      type="button"
      onClick={onClick}
    >
      {label}
      <span aria-hidden="true">
        {active ? (direction === "ascending" ? "↑" : "↓") : "↕"}
      </span>
    </button>
  );
}

function GameRow({
  cumulative,
  completionMode,
  game,
  maximumCumulativeSize,
  maximumRemainingHours,
  maximumStorageSize,
  mode,
  onCompare,
  onSelect,
  storageTargetSize,
}: {
  cumulative: CumulativeSize;
  completionMode: CompletionMode;
  game: LibraryGame;
  maximumCumulativeSize: number;
  maximumRemainingHours: number;
  maximumStorageSize: number;
  mode: SourceMode;
  onCompare: () => void;
  onSelect: () => void;
  storageTargetSize: number;
}) {
  const metrics = getGameMetrics(game, mode);
  const efficiencyBar = getEfficiencyBarWidths(metrics);
  const remainingTime = getRemainingTime(game, completionMode);
  const remainingMetrics = getRemainingMetrics(game, mode, completionMode);
  const remainingBar = getEfficiencyBarWidths(remainingMetrics);
  const remainingHoursBar = remainingTime
    ? getAdaptiveBarWidths(
        remainingTime.remainingHours,
        remainingTime.remainingHours,
        maximumRemainingHours,
      )
    : null;
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
            {game.appType !== "game" && game.appType !== "unknown" && (
              <em className="app-type-tag">{game.appType}</em>
            )}
          </small>
        </span>
      </span>
      <span className="numeric-cell" role="cell">
        <strong>{formatPlaytime(game.playtimeMinutes)}</strong>
        <small>lifetime</small>
      </span>
      <span className="numeric-cell remaining-hours-cell" role="cell">
        <strong>
          {remainingTime
            ? formatHltbDuration(remainingTime.remainingHours * 3600)
            : "—"}
        </strong>
        {remainingHoursBar && (
          <i
            className={`metric-bar remaining-hours-bar${remainingHoursBar.capped ? " capped" : ""}`}
            aria-label="Hours remaining"
          >
            <span
              className="remaining-hours-base"
              style={{ width: `${remainingHoursBar.basePercent}%` }}
            />
          </i>
        )}
        <small>
          {remainingTime
            ? (
                <>
                  left
                  {remainingTime.fallbackExplanation && (
                    <span
                      className="hltb-fallback-info"
                      aria-label={remainingTime.fallbackExplanation}
                      title={remainingTime.fallbackExplanation}
                    >
                      i
                    </span>
                  )}
                </>
              )
            : game.hltbStatus === "unmatched"
              ? "no match"
              : game.hltbStatus === "not_applicable"
                ? "not applicable"
                : game.hltbStatus === "matched"
                  ? (
                      <>
                        no completion data
                        <span
                          className="hltb-fallback-info"
                          aria-label={`${game.hltb?.gameName ?? game.name} was matched on HowLongToBeat, but it has no completion-time data.`}
                          title={`${game.hltb?.gameName ?? game.name} was matched on HowLongToBeat, but it has no completion-time data.`}
                        >
                          i
                        </span>
                      </>
                    )
                  : "pending"}
        </small>
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
      <span className="remaining-cell" role="cell">
        <strong>{formatEfficiency(remainingMetrics)}</strong>
        {remainingBar && (
          <i
            className="metric-bar remaining-bar"
            aria-label={
              remainingBar.rangePercent > 0
                ? "Remaining hours per GB range"
                : "Remaining hours per GB"
            }
          >
            <span
              className="remaining-base"
              style={{ width: `${remainingBar.basePercent}%` }}
            />
            {remainingBar.rangePercent > 0 && (
              <span
                className="remaining-range"
                style={{
                  left: `${remainingBar.basePercent}%`,
                  width: `${remainingBar.rangePercent}%`,
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
  completionMode,
  game,
  mode,
  probe,
  onClose,
}: {
  completionMode: CompletionMode;
  game: LibraryGame;
  mode: SourceMode;
  probe: DepotProbe | null;
  onClose: () => void;
}) {
  const metrics = getGameMetrics(game, mode);
  const remainingMetrics = getRemainingMetrics(game, mode, completionMode);
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
          {game.appType !== "game" && game.appType !== "unknown" && (
            <em className="app-type-tag">{game.appType}</em>
          )}
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
            <small>Played per GB</small>
            <strong>{formatEfficiency(metrics)}</strong>
          </article>
          <article>
            <small>Remaining per GB</small>
            <strong>{formatEfficiency(remainingMetrics)}</strong>
          </article>
        </div>
        <HltbSection game={game} />
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

function HltbSection({ game }: { game: LibraryGame }) {
  const [editing, setEditing] = useState(false);
  const [query, setQuery] = useState(game.name);
  const [results, setResults] = useState<HltbCandidate[]>([]);
  const [searching, setSearching] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function search() {
    const trimmed = query.trim();
    if (!trimmed) return;
    setSearching(true);
    setError(null);
    try {
      setResults(await invoke<HltbCandidate[]>("search_hltb", { query: trimmed }));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSearching(false);
    }
  }

  async function save(candidate: HltbCandidate | null) {
    setSaving(true);
    setError(null);
    try {
      await invoke("set_hltb_match", {
        appId: game.appId,
        candidate,
      });
      setEditing(false);
      setResults([]);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="hltb-section">
      <div className="hltb-heading">
        <div>
          <h3>HowLongToBeat</h3>
          <small>
            {game.hltb
              ? `Matched to ${game.hltb.gameName}`
              : game.hltbStatus === "not_applicable"
                ? "Not searched because this Steam item is not a game"
              : game.hltbStatus === "unmatched"
                ? "No confident match"
                : "Searching…"}
          </small>
        </div>
        {game.hltbStatus !== "not_applicable" && (
          <button type="button" onClick={() => setEditing((value) => !value)}>
            {editing ? "Cancel" : game.hltb ? "Change match" : "Find match"}
          </button>
        )}
      </div>
      {game.hltb && (
        <div className="hltb-times">
          <span>
            <small>Main Story</small>
            <strong>{formatHltbDuration(game.hltb.mainSeconds)}</strong>
          </span>
          <span>
            <small>Main + Extras</small>
            <strong>{formatHltbDuration(game.hltb.mainExtraSeconds)}</strong>
          </span>
          <span>
            <small>Completionist</small>
            <strong>{formatHltbDuration(game.hltb.completionistSeconds)}</strong>
          </span>
        </div>
      )}
      {editing && (
        <div className="hltb-editor">
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void search();
            }}
          >
            <input
              aria-label="Search HowLongToBeat"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            <button type="submit" disabled={searching || saving}>
              {searching ? "Searching…" : "Search"}
            </button>
          </form>
          {results.length > 0 && (
            <div className="hltb-results">
              {results.map((candidate) => (
                <button
                  type="button"
                  disabled={saving}
                  key={candidate.gameId}
                  onClick={() => void save(candidate)}
                >
                  <span>
                    <strong>{candidate.gameName}</strong>
                    <small>{candidate.platforms || "Platform unknown"}</small>
                  </span>
                  <b>{formatHltbDuration(candidate.mainExtraSeconds)}</b>
                </button>
              ))}
            </div>
          )}
          <button
            className="hltb-no-match"
            type="button"
            disabled={saving}
            onClick={() => void save(null)}
          >
            Mark as no match
          </button>
          {error && <p className="hltb-editor-error">{error}</p>}
        </div>
      )}
    </section>
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

function formatEfficiency(metrics: EfficiencyMetric) {
  if (!metrics || metrics.lowerHoursPerGiB === null) return "—";
  if (
    metrics.upperHoursPerGiB !== null &&
    Math.abs(metrics.upperHoursPerGiB - metrics.lowerHoursPerGiB) > 0.05
  ) {
    return `${metrics.lowerHoursPerGiB.toFixed(1)} → ${metrics.upperHoursPerGiB.toFixed(1)} h/GB`;
  }
  return `${metrics.lowerHoursPerGiB.toFixed(1)} h/GB`;
}

function formatHltbDuration(seconds: number | null) {
  if (seconds === null) return "—";
  const hours = seconds / 3600;
  if (hours < 10) return `${hours.toFixed(1)}h`;
  return `${Math.round(hours).toLocaleString()}h`;
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
