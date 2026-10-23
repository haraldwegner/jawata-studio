// A stand-in for the Tauri backend, so the real frontend can be RENDERED without the app (and without
// touching the one installed on this machine). Fixtures mirror the installed 4.4.7 / engine 4.4.12 window.
// URL: mock.html?tab=Memory&open=1&scenario=attention
//   tab      the nav tab to click after load (Dashboard, Memory, Reporting, Seats, Settings, Help)
//   open     1 = open every <details> after load (to see what the folds hold)
//   scenario attention = the Reporting view with something wrong
const q = new URLSearchParams(location.search);
const scenario = q.get("scenario") ?? "healthy";
const ws = ["javata-dev", "orb-strategy", "patterns"];
const projects = [];
const mk = (w, names) =>
  names.forEach((n) =>
    projects.push({ id: `${w}/${n}`, name: n, projectPath: `/work/${n}`, workspaceName: w }),
  );
mk("javata-dev", ["jawata-mcp", "javadoc-seat", "test-writer-seat", "architect-seat"]);
mk("orb-strategy", ["feed-client", "falcon", "alpaca-feed", "execsim-java", "massive-rest", "com-jats2-clicktrader", "com-jats2-gateways-gateway"]);
for (let i = 1; i <= 12; i++) mk("patterns", [`pattern-${i}`]);
const runtimeStatuses = Object.fromEntries(
  projects.map((p, i) => [
    p.id,
    {
      projectId: p.id, phase: "running", workspaceName: p.workspaceName, transport: "http", pid: 1182839 + (i % 3),
      workspaceDir: "/data/workspaces/" + p.workspaceName, logPath: "/tmp/x.log", runtimeLabel: "managed",
      resolvedJarPath: "/x.jar", serviceMode: "workspace", detail: "running - Live workspace runtime.",
    },
  ]),
);
const entry = (p) => ({ autoDetectedPath: p, manualOverridePath: null, effectivePath: p });
const settings = {
  version: 1, updatePolicy: "always", autoCheckForUpdates: true, manualFallbackJarPath: null, dataRoot: "/home/u/.cache/jawata-studio",
  globalRuntimeSource: { kind: "managed" }, useSystemTray: true, autostartOnBoot: false,
  mcpClientPaths: {
    cursor: entry("/home/u/.cursor/mcp.json"), claude: entry("/home/u/.claude.json"), codex: entry("/home/u/.codex/config.toml"),
    copilotCli: entry("/home/u/.copilot/mcp-config.json"), vscode: entry("/home/u/.config/Code/User/mcp.json"), grok: entry("/home/u/.grok/config.toml"),
  },
  mcpMergeMode: "safeMerge", mcpBackupBeforeWrite: true,
  deployTargets: { cursor: true, claude: true, codex: true, copilotCli: true, vscode: true, grok: true },
  releaseRepo: "haraldwegner/jawata-mcp", autoSeedOnDeploy: true, experienceStoreMode: "shared", memoryRoots: [], backupRetention: 5,
  experienceBackupDepth: null, experienceStoriesDir: null,
};
const dashboard = {
  bootstrap: {
    configDir: "/home/u/.config/jawata-studio", stateDir: "/home/u/.local/state/jawata-studio", cacheDir: "/home/u/.cache/jawata-studio",
    projectsFile: "/home/u/.config/jawata-studio/projects.json", settingsFile: "/home/u/.config/jawata-studio/settings.json",
    runtimeStateFile: "x", defaultDataRoot: "/home/u/.cache/jawata-studio", logDir: "x", transport: "http", healthStrategy: "x",
  },
  settings,
  releaseStatus: { kind: "ready", latestVersion: "4.4.12", defaultVersion: "4.4.12", checkedAt: null, updateAvailable: false, detail: "" },
  installedRuntime: { version: "4.4.12", installDir: "x", jarPath: "x", assetName: "x", installedAt: "x" },
  projects, runtimeStatuses, suggestedWorkspaceName: "javata-dev",
  servicesInventory: { available: true, services: [], detail: "" },
  workspaceRuntimeSettings: ws.map((w) => ({ workspaceName: w, maxHeapMb: null, effectiveMaxHeapMb: 2048, debuggable: false, debugPort: null })),
};
const bundles = ["org.jawata.core", "org.jawata.mcp", "org.jawata.boot", "com.example"];
const areas = Array.from({ length: 54 }, (_, i) => `org.jawata.mcp.area${i}`);
const knowledge = ws.map((w) => ({
  workspace: w, url: "http://x", reachable: true, error: null,
  stats: {
    total: 11622, by_status: { accepted: 342 }, by_language: { java: 11000 }, store: { file: "/home/u/.local/share/jawata/experience.mv.db", bytes: 119738777 },
    by_lane: { code: 11104, domain: 79, experience: 243, patterns: 189, rules: 7 },
    catalogue: { entries: 189, byNamespace: { "java-design-patterns": 187, "jawata-samples": 2 }, cures: { declared: 13, unresolved: 0, clean: true } },
    describing: {
      describedPerBundle: Object.fromEntries(bundles.map((b, i) => [b, 100 + i])), describedAreas: areas, acceptedRows: 400,
      acceptedSample: Array.from({ length: 10 }, (_, i) => ({
        symbol: `org.jawata.mcp.knowledge.StoreCorpus#m${i}`,
        summary: "Builds the next corpus snapshot from the previous one plus only the rows whose change mark moved, and gives up with null on any doubt.",
      })),
    },
  },
}));
const shapes = [
  ["experience/recall/KNOWLEDGE_UNAVAILABLE", 3159, "patterns"], ["experience/recall/KNOWLEDGE_UNAVAILABLE", 1253, "javata-dev"],
  ["inspect/source/PROJECT_LOADING", 1224, "patterns"], ["experience/describe/SYMBOL_NOT_FOUND", 999, "javata-dev"],
  ["experience/recall/KNOWLEDGE_UNAVAILABLE", 994, "orb-strategy"], ["inspect/source/PROJECT_LOADING", 420, "orb-strategy"],
  ["experience/record/INVALID_PARAMETER", 318, "javata-dev"], ["experience/nominate/KNOWLEDGE_UNAVAILABLE", 136, "javata-dev"],
];
const lane = () => ({ seat: "report", nudges: true, silenced: false, reminderReason: "", strikes: 0, remindersShown: 0, lastRemindedAtMillis: 0, nudgedShapes: [], postedShapes: [] });
const bad = scenario === "attention";
const field = {
  utilization: { jawataCalls: 3563259, shellFallbacks: 10060, slips: 3186, ungroundedReads: 6874, percent: 99.7, caveat: "Hook-scoped number: jawata only sees shell fallbacks in clients where its hooks run.", observerPresent: true },
  channels: [["guard", 59, 59], ["observer", 819, 0], ["primer", 47, 46], ["user-prompt", 303, 0]].map(([role, fired, emitted]) => ({
    role, fired, emitted, suppressed: {}, dead: bad && role === "primer", legitimatelyQuiet: false,
  })),
  deadChannels: bad ? ["primer"] : [], legitimatelyQuietChannels: [], silenceLogsRead: ["x"],
  workspaces: ws.map((w, i) => ({
    workspace: w, fieldDir: "x", lane: lane(),
    pile: {
      present: true, contract: 1, totalEvents: 100, failures: 50, successes: 50, badge: i === 0 ? 61 : 0, unreadableLines: 0,
      shapes: shapes.filter((s) => s[2] === w).map(([shape, count]) => ({ shape, count, posted: false, lastSeenMillis: 0 })),
    },
  })),
  badge: bad ? 61 : 0,
  canary: ws.map((w) => ({
    workspace: w, url: "x", recallOk: !(bad && w === "patterns"), recallDetail: bad && w === "patterns" ? "timed out" : "the store answered",
    compilerOk: true, compilerDetail: "the compiler resolved java.lang.String", green: !(bad && w === "patterns"), loading: false,
    loadingSinceMillis: null, checkedAtMillis: 0, recallMillis: 60,
  })),
  canaryHealth: bad ? "degraded" : "green",
  store: { health: "healthy", word: "answering", worstWorkspace: "", slowestMillis: 62, slowAboveMillis: 200, why: "" },
  recall: {
    present: true, applied: 0, rejected: 1, wouldBlock: 2381, blocked: 0, skipped: 4789, unavailable: 3168,
    coverage: "Claude Code on Linux/macOS only. Cursor fires no before-a-tool-call event and no agent-stop event, so a zero here means not observed.",
  },
};
const purposes = {
  architect: ["/refactor", "Reviews the design of your code and of each change against the target architecture, and proposes refactorings that keep behaviour unchanged. It advises only: nothing is applied without your yes.", "the sprint's DESIGN step, and every checkpoint as a watch-diff"],
  cataloguer: ["/catalogue", "Writes down what each class member and each package is for, so later questions about the code are answered from those descriptions instead of re-reading it. It changes no code, and a stopped run carries on where it left off.", "on a package nobody has described; resumes where it stopped"],
  debugger: ["/debug", "Diagnoses one reported bug: recalls what is already known about it, takes one measurement in the running program that tells the possible causes apart, and checks the fix. It replaces adding log lines to the code.", "the runtime reflex: a bug, a bad value, an NPE, instead of hand-adding log lines"],
  "javadoc-writer": ["/javadocs", "Documents public Java API that has no documentation, using only what the compiler confirms about it. Where the meaning cannot be told from the code it leaves a marked gap for you rather than guessing.", "the DOC gate: public API a change touched, before that change is done"],
  main: ["/main", "Runs work that has several steps as a workflow: it declares the steps, drives them one at a time, and refuses to call a step done until that step's check has actually run.", "whenever the work has more than one step"],
  profiler: ["/profile", "Measures a running Java program and names, method by method, where the time goes.", "the runtime reflex: performance, a hotspot"],
  report: ["/report", "Turns failures jawata has recorded on this machine into a bug report you post from your own GitHub account.", "when failures pile up and you want them filed"],
  review: ["/review", "Keeps your knowledge store healthy: it repairs the entries it can fix itself and shows you only what it cannot resolve.", "when the store has entries nobody uses"],
  rottenmeier: ["/rottenmeier", "Reads what made you angry or interrupt in the discussion, names what the agent did wrong, and proposes a rule that would have prevented it.", "after a session in which you corrected the agent"],
  "test-writer": ["/cover", "Writes tests that pin down how untested code behaves today, before a change that added behaviour is called done.", "the COVERAGE gate"],
  ux: ["/ux", "Looks at a changed screen as the person using it: can every part be reached, does each part help you decide or change something, and is each explanation enough to act on.", "any diff that changes a view"],
  "chief-engineer": [null, "Reads the code a change touches, decides which engine tool performs it and with which arguments, and issues each as an instruction with exactly one possible execution for the worker. It never changes code itself.", "when a change needs a decision about HOW the code is changed, before a worker acts"],
  observer: [null, "Watches a short window of an agent's recent tool calls beside the work, and says whether the agent is getting around a rule he gave. It never edits and is never waited on.", "beside the work, every few calls, started by the hook"],
  pm: [null, "Plans a piece of work and hands it to other seats with a brief, checks what comes back against the agreed checks, and records the result. It never writes code itself.", "when a seat's work needs a brain that is not this one"],
  "test-manager": [null, "Decides which tests a change must run and judges whether a red test is the change's fault.", "after a change, before the suite runs"],
  worker: [null, "Carries out one instruction exactly as written and reports what happened.", "when an instruction is issued"],
  "spec-editor": [null, "Writes the clean sprint spec from the raw working document.", "step one of /sprint"],
  "spec-auditor": [null, "Audits the clean spec against the raw document in a fresh context and refuses it if a requirement was dropped.", "step two of /sprint"],
};
const seats = Object.entries(purposes).map(([name, [command, purpose, fires]]) => ({
  name, command, pipeline: name.startsWith("spec-"), purpose, fires, brain: "claude", model: null, runsInAgent: !!command,
  enabled: true, deployable: !!command, clients: null, customised: false,
}));
const brains = [{ id: "claude", label: "Claude Code", installed: true, version: "x", login: "loggedIn", reason: null, loginCommand: "claude", models: ["opus", "sonnet"], hirable: true, takesModel: true, writesHeldByInstruction: false }];
const H = {
  get_dashboard: () => dashboard, get_quit_prompt_context: () => ({ workspaces: [] }), field_status: () => field,
  resolution_status: () => [], workspace_readability: () => ws.map((w) => ({ workspace: w, readable: true })),
  knowledge_status: () => knowledge, experience_verb: () => ({ ok: true }),
  seats_view: () => ({ seats, clients: [{ id: "claude", label: "Claude Code" }, { id: "cursor", label: "Cursor" }], errors: [] }),
  brains_status: () => brains, get_runtime_status: (a) => runtimeStatuses[a.projectId], get_services_inventory: () => dashboard.servicesInventory,
  probe_services: () => ({ ok: true, durationMs: 840, services: new Array(50).fill({ name: "x" }), detail: "" }),
};
let cb = 1;
window.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
  transformCallback: (f) => { const id = cb++; window[`_${id}`] = f; return id; },
  unregisterCallback: () => {},
  convertFileSrc: (p) => p,
  invoke: async (cmd, args) => {
    if (cmd.startsWith("plugin:")) return cmd.includes("listen") ? 1 : cmd.includes("version") ? "4.4.7" : null;
    const h = H[cmd];
    if (!h) { console.warn("mock: no handler for", cmd); return null; }
    return h(args ?? {});
  },
};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
window.addEventListener("load", () => {
  const tab = q.get("tab");
  setTimeout(() => {
    if (tab) [...document.querySelectorAll("nav .tab")].find((b) => b.textContent.trim() === tab)?.click();
    setTimeout(() => {
      if (q.get("open") === "1") document.querySelectorAll("details").forEach((d) => (d.open = true));
      document.title = "ready";
    }, 700);
  }, 700);
});
