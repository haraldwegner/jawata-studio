---
name: architect
kind: code
purpose: A fixture seat that stands in for a code seat in the deploy golden. It exists so that real seat edits never move the recorded bytes.
fires: never; a test reads it
effort: high
tools: [compile_workspace, find_references]
gates: []
ttl_secs: 600
max_iterations: 1
cost_budget_usd: 2.0
max_tool_steps: 80
---
This is the stance of the fixture code seat. It has two paragraphs so that line handling is
exercised.

Second paragraph: names `compile_workspace` and `find_references(` as the instruments.
