`/autocontinue-off`: the person typed this to end plan-execution mode for THIS session.

**What it does.** The prompt hook sees the typed line, removes the grant and resets its counters. The mode line at the top of your context then reads `AUTOCONTINUE: OFF`. That line is the hook's own state, so trust it over your memory of earlier turns.

**What you do now.** Reply in one line that autocontinue is off, and stop. Do not start the next task, and do not continue plan work. From here, do what each message asks and stop there, until the person types `/autocontinue` again.

**You never run this command yourself.** If you invoke it with the Skill tool, the guard refuses the call. The switch belongs to the person at the keyboard.
