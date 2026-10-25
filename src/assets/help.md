# jawata-studio Help

**jawata-studio** runs one JAWATA service per **workspace** (a named group of Java projects) and writes the connection into your AI agents (Cursor, Claude Code, Codex, GitHub Copilot CLI, VS Code, Grok), so they get compiler-accurate understanding of your code.

Every change an agent makes through jawata comes back with an undo handle: ask it to run `refactoring(action=undo, undoChangeId=…)` to put the code back.

Hover any control for what it does.

## Build a workspace

This is the job the studio is for. Five steps, all on the Dashboard:

1. **Create the workspace.** In **Workspaces** (left), click **+ New workspace…**, type a name, press Enter. Click a workspace to make it the one new projects join; it is highlighted.
2. **Add its projects.** Open **Add projects** (left).
   - **From a folder:** type or **Browse** to a Java project's folder, or a folder that holds several. The search starts at once (Enter also starts it) and lists every Maven, Gradle or Eclipse project under it, the folder itself included. Untick what you do not want, then **Import selected**.
   - **A plain source folder** with no Maven, Gradle or Eclipse file is not found by the search. Then **Add this folder as a project** adds it as it is.
   - **From a `.code-workspace` file or an IntelliJ project:** **Browse** to the file (for IntelliJ, the project folder or its `.idea/modules.xml`), **Discover**, then **Import selected**. Each IntelliJ module is offered: one with a Maven or Gradle build as that project, one without when its `.iml` names a source folder that exists.
3. **Check what landed.** In **Managed Projects** (right), open the workspace's card: each project shows its name and its folder. Drag a project onto another workspace on the left to move it. Right-click a project to start, stop, rename, move or delete it; right-click a workspace to start, stop, rename or delete it.
4. **Start it.** **Start workspace** on its card (it reads **Stop workspace** while the workspace runs), or **Start all**. The badge turns **RUNNING** once the service has loaded every project; hover it for the detail.
5. **Connect your agents.** In the **Agent deploy** panel under Managed Projects, **Deploy to agents** writes the jawata entries into the agents' config files. Restart or reload an agent to pick them up. **Remove from agents** takes exactly those entries out again.

**Deleting.** Every delete asks first and names what it removes; your source files on disk are never deleted. A running or starting workspace cannot be deleted, and neither can one of its projects: stop the workspace first. **Delete all workspaces** is at the bottom of the Workspaces panel.

## Dashboard

Everything above. **Reload all** stops every service and starts it again.

## Memory

The knowledge store behind what your agents recall. The top line says how many entries it holds and how big it is; the table names the store file and the workspaces that share it. **Load** reads your memory files into it, **Back up now** takes a copy, **Clean up** drops aged discarded entries and merges duplicates; **More** holds Reload status, Export, Import, Restore, Curation and Wipe. **Status** says what the last action did. **What the store holds** counts the entries of each kind, the pattern library's cures and how far the cataloguer has got. **Memory sources and store settings** sets which folders Load reads and how many backups are kept.

## Reporting

What jawata recorded about itself on this machine: shapes only (tool, error code, counts), never paths or text. The sentence on top says whether anything needs a look, and the line under it counts the failure shapes (kinds of error that repeat): how many are recorded, how many are filed and how many are still to report. A shape is worth reporting when it was seen 3 or more times and is not yet filed. **Details** lists every shape, most repeated first, and what happened when agents asked the memory store for knowledge. Type `/report` in any agent to file one from your own GitHub account.

## Seats

A seat is a narrow role an agent takes when you type its command, such as `/refactor` or `/debug`. The switch beside each takes its command out of your agents. Open a row for when it is meant to be used, which agent program and model a hire of it starts, which of your agents get its command, and **Reset to shipped**.

## Settings

**Engine**: whether the engine updates itself (**Update policy**), a link to the studio's releases, and **Test services**, which checks that the engine answers; **Advanced** holds where the engine is downloaded from or a local jar to use instead. **Agents**: which agents Deploy writes to, with each one's config file shown in the table (under **Advanced**: merge mode and a backup before each write). Grok receives jawata's tools but not its guard. Antigravity cannot connect jawata and is not supported. **This machine**: the data folder, tray, autostart, per-workspace memory limits, and cleanup.

## The tray and updates

Installing, per platform, is in the [README](https://github.com/haraldwegner/jawata-studio#installation); a newer studio is announced on the Releases page, linked from Settings → Engine. The tray needs a StatusNotifierItem host on Linux; vanilla GNOME needs `gnome-shell-extension-appindicator` once. The tray menu opens with Open dashboard, then the studio and engine versions, then your workspaces with the health of your services in their line.

## If something fails

Settings → This machine shows where the settings and logs are. **Remove from agents** and **Deploy to agents** again rewrite the entries. Keep **backup before MCP config write** on until you trust your layout.
