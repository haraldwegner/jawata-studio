import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";
import StartupProblemView from "./lib/components/StartupProblem.svelte";
import { startupProblem } from "./lib/api/tauri";

// A studio that could not read its files has no manager behind the page, so it is asked before anything is drawn.
// No answer (the page opened outside the studio) draws the app, which reports its own errors.
void startupProblem()
  .catch(() => null)
  .then((problem) => {
    const target = document.getElementById("app")!;
    if (problem) {
      mount(StartupProblemView, { target, props: { problem } });
    } else {
      mount(App, { target });
    }
  });
