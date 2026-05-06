# AGENTS.md

# Workflow Orchestration

## 1. Plan Mode Default
- Enter plan mode for ANY non-trivial task (3+ steps or architectural decisions)
- If something goes sideways, STOP and re-plan immediately — don't keep pushing
- Use plan mode for verification steps, not just building
- Write detailed specs upfront to reduce ambiguity

## 2. Subagent Strategy
- Use subagents liberally to keep main context window clean
- Offload research, exploration, and parallel analysis to subagents
- For complex problems, throw more compute at it via subagents
- One whole phase task per subagent for focused execution

## 3. Autonomous Documentation Maintenance
- Maintain documentation proactively after ANY implementation change — do not wait to be asked
- Before closing a task, sync all affected docs to the current code
- Treat the codebase as the single source of truth and align docs to it
- Remove obsolete, irrelevant, and duplicate content to keep docs clean
- Rewrite unclear sections when needed; do not just append patches onto stale documentation
- Ensure examples, commands, file paths, configs, and behavior descriptions reflect reality
- If documentation is already correct, explicitly confirm that it was checked

## 4. Verification Before Done
- Never mark a task complete without proving it works
- Diff behavior between main and your changes when relevant
- Ask yourself: "Would a staff engineer approve this?"
- Run tests, check logs, demonstrate correctness

## 5. Demand Elegance (Balanced)
- For non-trivial changes: pause and ask "is there a more elegant way?"
- If a fix feels hacky: know everything you know, implement the elegant solution
- Skip this for simple, obvious fixes — don't over-engineer
- Challenge your own work before presenting it

## 6. Autonomous Bug Fixing
- When given a bug report: just fix it. Don't ask for hand-holding
- Point at logs, errors, failing tests — then resolve them
- Zero context switching required from the user
- Go fix failing CI tests without being told how

## 7. Unit Test Discipline
- Unit tests must target isolated logic only: business rules, state changes, parsing, validation, serialization, and error handling
- Do not include UI flows, integration paths, real network/database/filesystem/device behavior, or end-to-end scenarios in unit tests
- Mock or fake external dependencies; keep tests fast, deterministic, and focused
- If UI/integration coverage is needed, label it separately and do not mix it into unit tests
- Run the relevant unit test command before closing and report the command/result