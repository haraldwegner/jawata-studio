/**
 * What the window says when the editor's folders match a jawata workspace only partly (Sprint 28h Stage 21 (j)).
 *
 * The studio asks, and changes nothing until the person answers. The text is built here, dependency free, so that
 * it is testable with `npm test` and so that the component only draws it.
 */

export interface PromptData {
  id: string;
  workspace: string;
  existing: string[];
  missing: string[];
  elsewhere: string[];
  folders: string[];
}

export interface PromptView {
  title: string;
  /** One plain sentence: what the question is. */
  question: string;
  /** The lists the person decides on, each with its heading; an empty list is not drawn. */
  sections: { heading: string; paths: string[] }[];
  /** The label of the two buttons. */
  update: string;
  decline: string;
}

/** The question that is shown: the first one open. The others wait their turn. */
export function nextPrompt(open: PromptData[]): PromptData | null {
  return open.length > 0 ? open[0] : null;
}

export function describePrompt(prompt: PromptData): PromptView {
  const count = prompt.missing.length;
  const sections = [
    { heading: `Already in ${prompt.workspace}`, paths: prompt.existing },
    { heading: "Missing, added by Update workspace", paths: prompt.missing },
    { heading: "In another workspace, left there", paths: prompt.elsewhere }
  ].filter((s) => s.paths.length > 0);
  return {
    title: "Update the workspace for this editor?",
    question:
      `The editor opened folders whose Java projects the workspace "${prompt.workspace}" holds only in part. ` +
      `Add the ${count} missing project${count === 1 ? "" : "s"} to it and start it? Nothing has been changed yet.`,
    sections,
    update: "Update workspace",
    decline: "Not now"
  };
}
