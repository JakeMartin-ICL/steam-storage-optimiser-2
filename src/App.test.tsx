import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App, {
  buildCumulativeSizes,
  filterAndSortGames,
  formatByteRange,
  formatBytes,
  getAdaptiveBarWidths,
  getCumulativeBarWidths,
  getEfficiencyBarWidths,
  getGameMetrics,
  getHltbTarget,
  getRemainingMetrics,
  getRemainingTime,
  getStorageScaleMaximum,
  getStorageBarWidths,
  gameArtworkUrl,
  hasLargeSizeDiscrepancy,
  initialAuthView,
  type AuthView,
  type LibraryGame,
} from "./App";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

const mockedInvoke = vi.mocked(invoke);
const mockedOpen = vi.mocked(open);
const gib = 1024 ** 3;
const games: LibraryGame[] = [
  {
    appId: 1,
    name: "Installed Hero",
    appType: "game",
    playtimeMinutes: 600,
    sharedOnly: false,
    installed: true,
    localSizeBytes: 2 * gib,
    depotSizeBytes: 3 * gib,
    depotStatus: "available",
    depotExact: true,
    depotCount: 1,
    depotOs: "macos",
    currentOsSupported: true,
    depotWarnings: [],
    depotError: null,
    communitySizeBytes: 4 * gib,
    hltb: {
      gameId: 101,
      gameName: "Installed Hero",
      mainSeconds: 15 * 3600,
      mainExtraSeconds: 30 * 3600,
      completionistSeconds: 50 * 3600,
      steamAppId: 1,
      matchMethod: "steam_app_id",
    },
    hltbStatus: "matched",
  },
  {
    appId: 2,
    name: "Shared Quest",
    appType: "game",
    playtimeMinutes: null,
    sharedOnly: true,
    installed: false,
    localSizeBytes: null,
    depotSizeBytes: 4 * gib,
    depotStatus: "available",
    depotExact: false,
    depotCount: 2,
    depotOs: "windows",
    currentOsSupported: false,
    depotWarnings: ["Family owner selected"],
    depotError: null,
    communitySizeBytes: 8 * gib,
    hltb: null,
    hltbStatus: "pending",
  },
];
const completeView: AuthView = {
  ...initialAuthView,
  phase: "complete",
  message: "Library updated.",
  libraryCount: games.length,
  games,
  savedLogin: true,
  profile: {
    displayName: "Jake",
    avatarUrl: "https://avatars.steamstatic.com/fixture_medium.jpg",
  },
};

describe("product UI", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedOpen.mockReset();
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") return initialAuthView;
      if (command === "has_saved_login") return false;
      return undefined;
    });
  });

  it("offers a persisted manual Steam location when discovery fails", async () => {
    const user = userEvent.setup();
    mockedOpen.mockResolvedValue("/custom/Steam");
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") return initialAuthView;
      if (command === "has_saved_login") return false;
      if (command === "get_steam_location") {
        return { path: null, source: null };
      }
      if (command === "set_steam_location") {
        return { path: "/custom/Steam", source: "saved" };
      }
      return undefined;
    });
    render(<App />);

    await user.click(
      await screen.findByRole("button", { name: "Locate Steam" }),
    );

    expect(mockedOpen).toHaveBeenCalledWith(
      expect.objectContaining({
        directory: true,
        multiple: false,
      }),
    );
    expect(mockedInvoke).toHaveBeenCalledWith("set_steam_location", {
      path: "/custom/Steam",
    });
  });

  it("explains the product without implementation-detail badges", () => {
    render(<App />);

    expect(
      screen.getByRole("heading", {
        name: "Compare games by size and playtime.",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "See storage estimates and lifetime playtime for every game in your library.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("No API key")).not.toBeInTheDocument();
    expect(screen.queryByText("Read-only")).not.toBeInTheDocument();
  });

  it("starts QR login through the native boundary", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: /Connect with Steam/ }));

    expect(mockedInvoke).toHaveBeenCalledWith("start_qr_login", {
      contributeCommunitySizes: true,
    });
  });

  it("allows community contributions to be disabled before login", async () => {
    const user = userEvent.setup();
    render(<App />);

    const contribution = screen.getByRole("switch", {
      name: "Contribute installed sizes",
    });
    expect(contribution).toBeChecked();
    expect(
      screen.getByRole("button", { name: "About community contributions" }),
    ).toHaveAttribute("data-tooltip", expect.stringContaining("personal data"));

    await user.click(contribution);
    await user.click(screen.getByRole("button", { name: /Connect with Steam/ }));

    expect(mockedInvoke).toHaveBeenCalledWith("start_qr_login", {
      contributeCommunitySizes: false,
    });
  });

  it("searches, opens details, and disconnects", async () => {
    const user = userEvent.setup();
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") return completeView;
      return undefined;
    });
    render(<App />);

    expect(
      await screen.findByRole("heading", { name: "Your library" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Installed Hero")).toBeInTheDocument();
    expect(screen.getByText("Shared Quest")).toBeInTheDocument();
    expect(screen.getByText("pending")).toBeInTheDocument();
    expect(screen.queryByText("no match")).not.toBeInTheDocument();
    expect(screen.getByText("App 1")).toBeInTheDocument();
    expect(screen.getByText("App 2")).toBeInTheDocument();
    expect(screen.getByText("Jake")).toBeInTheDocument();
    expect(
      document.querySelector(`img[src="${gameArtworkUrl(1)}"]`),
    ).toBeInTheDocument();
    expect(
      screen.getByTitle(/using the Windows depot estimate/),
    ).toHaveTextContent("only");

    await user.type(
      screen.getByRole("textbox", { name: "Search library" }),
      "hero",
    );
    expect(screen.getByText("Installed Hero")).toBeInTheDocument();
    expect(screen.queryByText("Shared Quest")).not.toBeInTheDocument();

    await user.clear(screen.getByRole("textbox", { name: "Search library" }));
    await user.click(screen.getByText("Installed Hero"));
    expect(
      screen.getByRole("heading", { name: "Installed Hero" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Exact")).toBeInTheDocument();
    expect(screen.getByText("File-level manifest")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close details" }));

    await user.click(screen.getByText("Shared Quest"));
    expect(
      screen.getByText("Not available for this operating system"),
    ).toBeInTheDocument();
    expect(screen.getByText("Windows · 2 depot manifest summary")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close details" }));

    expect(screen.queryByText("Insights")).not.toBeInTheDocument();
    expect(screen.queryByText("Local data connected")).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Disconnect Steam account" }),
    );
    expect(mockedInvoke).toHaveBeenCalledWith("forget_saved_login");
  });

  it("defaults to depot sizes and can open comparison from a discrepancy", async () => {
    const user = userEvent.setup();
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") return completeView;
      return undefined;
    });
    render(<App />);

    const source = await screen.findByRole("combobox", {
      name: "Size source",
    });
    expect(source).toHaveValue("depot");
    expect(screen.getByText("Community size")).toBeInTheDocument();
    expect(screen.getByText("8.00 GiB")).toBeInTheDocument();
    expect(
      screen.getByText(/May exclude launchers, bootstrapping/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/May be out of date or based on a different operating system/),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Compare sources" }),
    );
    expect(source).toHaveValue("compare");
  });

  it("can hide Steam Family games", async () => {
    const user = userEvent.setup();
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") return completeView;
      return undefined;
    });
    render(<App />);
    await screen.findByText("Shared Quest");

    await user.click(screen.getByRole("checkbox", { name: "Hide shared" }));

    await waitFor(() =>
      expect(screen.queryByText("Shared Quest")).not.toBeInTheDocument(),
    );
    expect(screen.getByText("1 of 2 games").closest("nav")).not.toBeNull();
  });

  it("can manually correct a HowLongToBeat match", async () => {
    const user = userEvent.setup();
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") return completeView;
      if (command === "search_hltb") {
        return [
          {
            gameId: 202,
            gameName: "Installed Hero: Definitive Edition",
            mainSeconds: 20 * 3600,
            mainExtraSeconds: 35 * 3600,
            completionistSeconds: 60 * 3600,
            steamAppId: null,
            platforms: "PC",
            similarity: 0.9,
          },
        ];
      }
      return undefined;
    });
    render(<App />);

    await user.click(await screen.findByText("Installed Hero"));
    await user.click(screen.getByRole("button", { name: "Change match" }));
    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.click(
      await screen.findByRole("button", {
        name: /Installed Hero: Definitive Edition/,
      }),
    );

    expect(mockedInvoke).toHaveBeenCalledWith("set_hltb_match", {
      appId: 1,
      candidate: expect.objectContaining({ gameId: 202 }),
    });
  });

  it("distinguishes a match with no HLTB completion data", async () => {
    const noTimesGame: LibraryGame = {
      ...games[0],
      hltb: {
        ...games[0].hltb!,
        gameName: "#SelfieTennis",
        mainSeconds: null,
        mainExtraSeconds: null,
        completionistSeconds: null,
      },
      hltbStatus: "matched",
    };
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") {
        return { ...completeView, games: [noTimesGame], libraryCount: 1 };
      }
      return undefined;
    });
    render(<App />);

    expect(await screen.findByText("no completion data")).toBeInTheDocument();
    expect(
      screen.getByLabelText(
        "#SelfieTennis was matched on HowLongToBeat, but it has no completion-time data.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("no match")).not.toBeInTheDocument();
  });

  it("can hide games incompatible with the current OS", async () => {
    const user = userEvent.setup();
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") return completeView;
      return undefined;
    });
    render(<App />);
    await screen.findByText("Shared Quest");

    await user.click(
      screen.getByRole("checkbox", { name: "Hide incompatible" }),
    );

    await waitFor(() =>
      expect(screen.queryByText("Shared Quest")).not.toBeInTheDocument(),
    );
  });

  it("hides Steam software by default and can include it", async () => {
    const user = userEvent.setup();
    const software = {
      ...games[1],
      appId: 3,
      name: "Benchmark Tool",
      appType: "application",
      sharedOnly: false,
      currentOsSupported: true,
      hltbStatus: "not_applicable" as const,
    };
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") {
        return { ...completeView, games: [...games, software], libraryCount: 3 };
      }
      return undefined;
    });
    render(<App />);

    await screen.findByText("Installed Hero");
    expect(screen.queryByText("Benchmark Tool")).not.toBeInTheDocument();
    expect(screen.getByText("Total games").parentElement).toHaveTextContent("2");
    await user.click(
      screen.getByRole("checkbox", { name: "Show software & tools" }),
    );
    expect(screen.getByText("Benchmark Tool")).toBeInTheDocument();
    expect(screen.getByText("application")).toBeInTheDocument();
    expect(screen.getByText("Total items").parentElement).toHaveTextContent("3");
  });

  it("sorts by clicking table headers and toggles direction", async () => {
    const user = userEvent.setup();
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") return completeView;
      return undefined;
    });
    render(<App />);

    const storageHeader = await screen.findByRole("columnheader", {
      name: /Storage/,
    });
    const playtimeHeader = screen.getByRole("columnheader", {
      name: "Playtime",
    });
    const remainingHeader = screen.getByRole("columnheader", {
      name: "Remaining",
    });
    const headers = screen.getAllByRole("columnheader");
    expect(headers.indexOf(remainingHeader)).toBe(headers.indexOf(playtimeHeader) + 1);
    expect(screen.queryByRole("combobox", { name: "Sort library" })).toBeNull();
    await user.click(storageHeader);
    expect(storageHeader).toHaveAttribute("aria-sort", "descending");
    await user.click(storageHeader);
    expect(storageHeader).toHaveAttribute("aria-sort", "ascending");
  });

  it("keeps compact library controls in the sidebar", async () => {
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") return completeView;
      return undefined;
    });
    render(<App />);

    await screen.findByText("Installed Hero");
    const sizeTarget = screen.getByRole("slider", {
      name: "Library size target",
    });
    const completionTarget = screen.getByRole("slider", {
      name: "HowLongToBeat completion level",
    });
    expect(sizeTarget.closest("aside")).not.toBeNull();
    expect(completionTarget.closest("aside")).not.toBeNull();
    expect(completionTarget).toHaveStyle("--completion-fill: 50%");

    fireEvent.change(completionTarget, { target: { value: "0" } });
    expect(completionTarget).toHaveStyle("--completion-fill: 0%");

    expect(screen.getByText("2 games").closest("nav")).not.toBeNull();
    expect(
      screen.queryByText(/Installed games always use Steam/),
    ).not.toBeInTheDocument();
    const storageHeader = screen.getByRole("columnheader", {
      name: "Storage",
    });
    expect(storageHeader.querySelector(".header-info")).toHaveAttribute(
      "data-tooltip",
      expect.stringContaining("exact local size"),
    );
  });

  it("shows non-blocking library-wide depot progress", async () => {
    mockedInvoke.mockImplementation(async (command) => {
      if (command === "get_auth_state") {
        return {
          ...completeView,
          depotProgress: {
            completed: 50,
            total: 200,
            available: 44,
            unavailable: 6,
          },
        };
      }
      return undefined;
    });
    render(<App />);

    expect(await screen.findByText("Measuring Steam depots")).toBeInTheDocument();
    expect(screen.getByText("50 / 200")).toBeInTheDocument();
    expect(
      screen.getByText(
        "You can use the library while estimates arrive in the background.",
      ),
    ).toBeInTheDocument();
  });
});

describe("library calculations", () => {
  it("flags discrepancies over sixty-six percent with a hundred MiB floor", () => {
    expect(
      hasLargeSizeDiscrepancy({
        ...games[1],
        depotSizeBytes: 100 * 1024 ** 2,
        communitySizeBytes: 200 * 1024 ** 2,
      }),
    ).toBe(true);
    expect(
      hasLargeSizeDiscrepancy({
        ...games[1],
        depotSizeBytes: 100 * 1024 ** 2,
        communitySizeBytes: 199 * 1024 ** 2,
      }),
    ).toBe(false);
  });

  it("uses exact local size for installed games", () => {
    const metrics = getGameMetrics(games[0], "compare");

    expect(metrics?.lowerSizeBytes).toBe(2 * gib);
    expect(metrics?.sources).toEqual(["local"]);
    expect(metrics?.lowerHoursPerGiB).toBe(5);
  });

  it("subtracts played time from the selected completion target", () => {
    const metrics = getRemainingMetrics(games[0], "depot", "main_extra");

    expect(metrics?.remainingHours).toBe(20);
    expect(metrics?.lowerHoursPerGiB).toBe(10);
    expect(metrics?.upperHoursPerGiB).toBe(10);
  });

  it("calculates remaining time without requiring a storage estimate", () => {
    const gameWithoutSize = {
      ...games[0],
      installed: false,
      localSizeBytes: null,
      depotSizeBytes: null,
      communitySizeBytes: null,
    };

    expect(getRemainingTime(gameWithoutSize, "main_extra")?.remainingHours).toBe(
      20,
    );
    expect(getRemainingMetrics(gameWithoutSize, "depot", "main_extra")).toBeNull();
  });

  it("uses the closest available HLTB estimate for each completion level", () => {
    const estimate = {
      ...games[0].hltb!,
      mainSeconds: 2 * 3600,
      mainExtraSeconds: null,
      completionistSeconds: 10 * 3600,
    };

    expect(getHltbTarget(estimate, "main")).toEqual({
      seconds: 2 * 3600,
      fallbackExplanation: null,
    });
    expect(getHltbTarget(estimate, "main_extra")).toEqual({
      seconds: 6 * 3600,
      fallbackExplanation:
        "Main + Extras is unavailable, so this uses the midpoint between Main Story and Completionist.",
    });
    expect(getHltbTarget(estimate, "completionist")).toEqual({
      seconds: 10 * 3600,
      fallbackExplanation: null,
    });
  });

  it("falls outward through the remaining HLTB categories", () => {
    const completionistOnly = {
      ...games[0].hltb!,
      mainSeconds: null,
      mainExtraSeconds: null,
      completionistSeconds: 10 * 3600,
    };
    const mainOnly = {
      ...completionistOnly,
      mainSeconds: 2 * 3600,
      completionistSeconds: null,
    };

    expect(getHltbTarget(completionistOnly, "main")?.seconds).toBe(
      10 * 3600,
    );
    expect(getHltbTarget(completionistOnly, "main_extra")?.seconds).toBe(
      10 * 3600,
    );
    expect(getHltbTarget(mainOnly, "completionist")?.seconds).toBe(2 * 3600);
  });

  it("keeps a match even when HLTB has no completion data", () => {
    const noTimes = {
      ...games[0].hltb!,
      mainSeconds: null,
      mainExtraSeconds: null,
      completionistSeconds: null,
    };

    expect(getHltbTarget(noTimes, "main_extra")).toBeNull();
  });

  it("turns differing estimates into a range", () => {
    const metrics = getGameMetrics(games[1], "compare");

    expect(metrics?.lowerSizeBytes).toBe(4 * gib);
    expect(metrics?.upperSizeBytes).toBe(8 * gib);
    expect(metrics?.sources).toEqual(["depot", "community"]);
    expect(metrics?.collapsedComparison).toBe(false);
  });

  it("collapses estimates within fifteen percent to the depot value", () => {
    const metrics = getGameMetrics(
      {
        ...games[1],
        depotSizeBytes: 10 * gib,
        communitySizeBytes: 11.4 * gib,
      },
      "compare",
    );

    expect(metrics?.lowerSizeBytes).toBe(10 * gib);
    expect(metrics?.upperSizeBytes).toBe(10 * gib);
    expect(metrics?.collapsedComparison).toBe(true);
    expect(metrics?.sources).toEqual(["depot", "community"]);
  });

  it("also collapses estimates within one hundred MiB", () => {
    const depot = 500 * 1024 ** 2;
    const metrics = getGameMetrics(
      {
        ...games[1],
        depotSizeBytes: depot,
        communitySizeBytes: depot + 100 * 1024 ** 2,
      },
      "compare",
    );

    expect(metrics?.lowerSizeBytes).toBe(depot);
    expect(metrics?.upperSizeBytes).toBe(depot);
    expect(metrics?.collapsedComparison).toBe(true);
  });

  it("splits the efficiency bar into conservative and range segments", () => {
    const metrics = getGameMetrics(
      {
        ...games[1],
        playtimeMinutes: 600,
      },
      "compare",
    );

    expect(getEfficiencyBarWidths(metrics)).toEqual({
      basePercent: 3.75,
      rangePercent: 3.75,
    });
  });

  it("does not add a second bar colour for a collapsed comparison", () => {
    const metrics = getGameMetrics(
      {
        ...games[1],
        playtimeMinutes: 600,
        depotSizeBytes: 10 * gib,
        communitySizeBytes: 11 * gib,
      },
      "compare",
    );

    expect(getEfficiencyBarWidths(metrics)?.rangePercent).toBe(0);
  });

  it("shows storage uncertainty after the lower estimate", () => {
    const metrics = getGameMetrics(games[1], "compare");

    expect(getStorageBarWidths(metrics, 10 * gib)).toEqual({
      basePercent: 40,
      rangePercent: 40,
      capped: false,
    });
  });

  it("keeps the true maximum for a uniform storage distribution", () => {
    const sizes = Array.from({ length: 20 }, (_, index) => (index + 1) * gib);

    expect(getStorageScaleMaximum(sizes)).toBe(20 * gib);
  });

  it("caps a separated storage outlier without changing the linear scale", () => {
    const ordinarySizes = Array.from(
      { length: 20 },
      (_, index) => (index + 1) * gib,
    );
    const scaleMaximum = getStorageScaleMaximum([
      ...ordinarySizes,
      200 * gib,
    ]);

    expect(scaleMaximum).toBe(31 * gib);
    expect(
      getStorageBarWidths(
        {
          lowerSizeBytes: 200 * gib,
          upperSizeBytes: 200 * gib,
          lowerHoursPerGiB: 1,
          upperHoursPerGiB: 1,
          sources: ["depot"],
          fallback: false,
          collapsedComparison: false,
        },
        scaleMaximum,
      ),
    ).toEqual({
      basePercent: 100,
      rangePercent: 0,
      capped: true,
    });
    expect(getAdaptiveBarWidths(200, 200, scaleMaximum / gib)).toEqual({
      basePercent: 100,
      rangePercent: 0,
      capped: true,
    });
  });

  it("splits cumulative storage into the previous total and new game", () => {
    const widths = getCumulativeBarWidths(
      {
        lowerSizeBytes: 6 * gib,
        upperSizeBytes: 10 * gib,
        unknownCount: 0,
      },
      4 * gib,
      5 * gib,
      6 * gib,
    );

    expect(widths?.previousPercent).toBeCloseTo(100 / 3);
    expect(widths?.addedPercent).toBeCloseTo(200 / 3);
    expect(widths?.overTarget).toBe(true);
  });

  it("uses the library target as the cumulative scale until it is exceeded", () => {
    const widths = getCumulativeBarWidths(
      {
        lowerSizeBytes: 4 * gib,
        upperSizeBytes: 4 * gib,
        unknownCount: 0,
      },
      2 * gib,
      5 * gib,
      6 * gib,
    );

    expect(widths).toEqual({
      previousPercent: 40,
      addedPercent: 40,
      overTarget: false,
    });
  });

  it("filters shared games", () => {
    const result = filterAndSortGames(games, {
      query: "",
      scope: "all",
      hideShared: true,
      hideIncompatible: false,
      sort: "size",
      sourceMode: "compare",
    });

    expect(result.map((game) => game.name)).toEqual(["Installed Hero"]);
  });

  it("filters non-game Steam applications unless explicitly included", () => {
    const software = {
      ...games[0],
      appType: "application",
    };
    const result = filterAndSortGames([games[0], software], {
      query: "",
      scope: "all",
      hideShared: false,
      hideIncompatible: false,
      showNonGames: false,
      sort: "name",
      sourceMode: "depot",
    });

    expect(result).toEqual([games[0]]);
  });

  it("recalculates cumulative ranges from the visible games", () => {
    const allSizes = buildCumulativeSizes(games, "compare");
    const compatibleGames = filterAndSortGames(games, {
      query: "",
      scope: "all",
      hideShared: false,
      hideIncompatible: true,
      sort: "name",
      sourceMode: "compare",
    });
    const compatibleSizes = buildCumulativeSizes(compatibleGames, "compare");

    expect(allSizes[1]).toEqual({
      lowerSizeBytes: 6 * gib,
      upperSizeBytes: 10 * gib,
      unknownCount: 0,
    });
    expect(compatibleSizes).toEqual([
      {
        lowerSizeBytes: 2 * gib,
        upperSizeBytes: 2 * gib,
        unknownCount: 0,
      },
    ]);
  });

  it("uses binary storage units", () => {
    expect(formatBytes(gib)).toBe("1.00 GiB");
  });

  it("does not share a unit across a binary-unit boundary", () => {
    expect(formatByteRange(900 * 1024 ** 2, gib)).toBe(
      "900.00 MiB → 1.00 GiB",
    );
  });
});
