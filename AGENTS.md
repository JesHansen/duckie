# Working on Duckie

These instructions apply to human contributors and coding agents, regardless of provider or model.

## Read the document that owns the information

- [README.md](README.md): running and using Duckie, development checks, and licensing.
- [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md): current owner decisions, accepted scope, prioritized backlog, known defects, and implementation cautions.
- [PERFORMANCE.md](PERFORMANCE.md): benchmark commands, measurements, and their limitations.
- [ARCHITECTURE.md](ARCHITECTURE.md) and [UI_DESIGN.md](UI_DESIGN.md): original design proposals. Later owner decisions in the status document supersede conflicting proposals.

## Working approach

- Follow the user's current task. A backlog or checkpoint does not authorize automatically starting another task or resuming paused work.
- Use time and token budgets efficiently. Choose available tools and models according to task complexity; no particular provider, model, or orchestration role is required. Delegate bounded, independent work when permitted by the current session and likely to reduce total effort; otherwise work directly.
- Inspect the working tree before editing and preserve unrelated changes.
- Keep changes within the relevant module boundaries and preserve the local-only product contract. Do not introduce unsolicited network activity.
- Validate code changes with the appropriate checks from the README. For documentation-only changes, check accuracy, links, and the diff; a full application build is usually unnecessary.
- Distinguish measured results, owner acceptance, reported findings, and unverified hypotheses. Do not present a partial benchmark as a full release-gate measurement.

## Keep documentation consolidated

Update the document that owns the information instead of creating another session log, agent-specific instruction file, or duplicate backlog. Link to measurements rather than copying their tables. Keep durable technical findings in the status document and reproducible benchmark details in the performance document. Date validation snapshots so test counts and package claims are not mistaken for current verification.
