# Rules all AI Agents must follow when building within this repo

1. Simplicity and maintainability are extremely important. Nuances, quirks, and edge cases should be compartmentalized from the core logic; which itself should be clear and consistent.

2. Document all publicly exported methods and types. Documentation should be succinct, outline the purpose, and note any non-obvious semantics or caveats.

3. In code documentation, avoid referencing code or behavior that used to be there, but no longer is; e.g. "this method replaces methods X and Y".

4. Avoid redundant documentation. Specify only non-obvious caveats, and high level abstractions, decisions, and justifications. Do not reiterate information that could be easily gleaned by just looking at the code.

5. After finishing a coding task, perform a simplification pass - go through all the code added and look for opportunities to simplify it and reduce the amount of code that needs to be created and maintained.

6. After finishing a coding task, perform a re-documentation pass - give a subagent with no context the code written and instruct it to go through all the comments and point out anything that doesn't make sense or is unclear due to lack of context from the conversation in which it was written. The idea is to emulate what someone what might see if they are looking at this code for the first time. Clarify (or add) comments for anything that is unclear or confusing.

7. In larger scoped features (if unsure whether any given task qualifies, just ask), debugging and diagnostics (both instrumentation and tooling) become a first class concern that is similar in importance to any major feature (i.e. "table stakes"). Traces, logs, test harnesses, visualizers, and any other debugging instruments must be designed and developed with the same attention to completeness, reliability, and performance as any and all other features. Consequently, the user should be involved in the design/scoping and code architecture, just as with other features. A predictable and consistent performance impact is acceptable, as long as the offending instrumentation can be optionally disabled safely in production environments (without invalidating any assertions or observations obtained with it enabled during development). All foreseeable error conditions or invalid states should be handled and reported gracefully, with sufficient context information to either present the end-user with clear options of actions to take to resolve the condition, or present the developer with sufficient state and trace data to change the implementation to avoid the situation entirely in the future. Unforeseen errors or invalid states should be reproducible in a development environment (with appropriate mocking/stubbing of external systems) from the logging/tracing of the program: which must include sufficient information about interactions with external systems to replicate their behavior in the given scenario.

8. Follow the existing formatting and layout of docs and code. Design decisions, architecture and research go in `docs/`, one doc per topic, in the format set out in `docs/README.md`; what is done and what is left goes in `ROADMAP.md`; how to develop and debug goes in `DEVELOPMENT.md`.
