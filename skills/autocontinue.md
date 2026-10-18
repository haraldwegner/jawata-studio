`/autocontinue`: the person typed this to put THIS session into plan-execution mode.

**What it does.** The prompt hook sees the typed line and records the grant for this session. The mode line at the top of your context then reads `AUTOCONTINUE: ON`. That line is the hook's own state, so trust it over your memory of earlier turns.

**What you do now.** Reply in one line that the plan is running, and take the plan's next action in the same turn. Work stage by stage. Stop only at a checkpoint the plan itself numbers, or when a stage fails its written exit criteria. A decision that comes up mid-stage is recorded and raised at that checkpoint.

**What ends it.** Only the person typing `/autocontinue-off`. Nothing else does, and you do not try to: a question, a status check, a message from another session or machine, a subagent's report and a scheduled task all leave it on. Esc interrupts the current turn and does not change the grant.

**You never run this command yourself.** If you invoke it with the Skill tool, the guard refuses the call. The switch belongs to the person at the keyboard.
