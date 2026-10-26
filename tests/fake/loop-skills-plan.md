<!-- loop-plan
planId: skills-adopt
-->
# WS v6 skills opening fixture

## Loop config
- **Integration branch:** `ws-revamp`
## Repositories
### `rstagi/dotfile`
- **Verify:** `true`

## Milestone 1 — Spikes settle the v6 unknowns (roadmap phase 0) `[archived]`
- **Integration branch (rstagi/dotfile):** `ws-revamp`
- **Repositories:** `rstagi/dotfile`
- **Phases (all done):** 1, 2, 3, 4, 5, 6, 7, 8, 9
- **Gate:** Phase 9
- **Archive:** [milestones/m1.md](milestones/m1.md)

## Milestone 2 — Rust wsd replaces the zsh ws on one machine (roadmap phase 1) `[archived]`
- **Integration branch (rstagi/dotfile):** `feat/wsd-local-daemon`
- **Repositories:** `rstagi/dotfile`
- **Phases (all done):** 69, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 67, 57, 58, 22, 23, 24, 25, 26, 27, 28, 56, 29, 30, 31, 59, 32, 33, 34, 35, 36, 64, 37, 38
- **Gate:** Phase 38
- **Archive:** [milestones/m2.md](milestones/m2.md)

## Milestone 3 — Local telemetry, quota visibility and the milestone 2 fixes on wsd (roadmap phase 2) `[archived]`
- **Integration branch (rstagi/dotfile):** `feat/wsd-telemetry`
- **Repositories:** `rstagi/dotfile`
- **Phases (all done):** 71, 105, 106, 107, 74, 72, 73, 39, 40, 41, 42, 43, 44, 45, 46, 47, 70, 48, 49, 108, 54, 55
- **Gate:** Phase 55
- **Archive:** [milestones/m3.md](milestones/m3.md)

## Milestone 4 — Sessions run without a repo or a workspace (D18) `[archived]`
- **Integration branch (rstagi/dotfile):** `feat/ws-sessions-without-repos`
- **Repositories:** `rstagi/dotfile`
- **Phases (all done):** 118, 119, 120, 75, 76, 147, 77, 148, 78, 79, 80, 81, 82, 83, 84
- **Gate:** Phase 84
- **Archive:** [milestones/m4.md](milestones/m4.md)

## Milestone 5 — The Loop gets its core wsd tools and guardrails next to the old engine (D19, D20, D21) `[archived]`
- **Integration branch (rstagi/dotfile):** `feat/loop-skills-on-ws`
- **Repositories:** `rstagi/dotfile`
- **Phases (all done):** 149, 85, 86, 101, 102, 87, 109, 110, 117, 133, 89, 94, 88, 90, 91, 92, 103, 111, 112, 113, 114, 134, 146
- **Gate:** Phase 146
- **Archive:** [milestones/m5.md](milestones/m5.md)

## Milestone 6 — The Loop gets a skills engine next to the old engine (D19, D21)
- **Integration branch (rstagi/dotfile):** `feat/loop-skills-engine`

### Phase 137 — Open a skills loop from its plan or from an advanced loop dir `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/loop-milestone-open`
- **Depends on:** Phase 146

### Phase 138 — Park a skills loop at a merge gate `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/loop-milestone-park`
- **Depends on:** Phase 137

### Phase 139 — Resume a skills loop after the merge of its gate PRs `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/loop-milestone-resume`
- **Depends on:** Phase 138

### Phase 150 — Take the phase definitions of a loop from its latest plan snapshot `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `fix/wsd-loop-phase-snapshot`
- **Depends on:** Phase 146

### Phase 104 — Add the skills engine switch to loop_start `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/wsd-engine-switch`
- **Depends on:** Phase 150

### Phase 144 — Open and resolve an accept gate with tools `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/loop-accept-gate`
- **Depends on:** Phase 104, Phase 139

### Phase 136 — Record a terminal result for each review role `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/loop-review-results`
- **Depends on:** Phase 146

### Phase 93 — Add the loop-review skill `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/skill-loop-review`
- **Depends on:** Phase 136

### Phase 145 — Make a lane merge safe to retry and to undo `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `fix/loop-merge-retry-undo`
- **Depends on:** Phase 146

### Phase 140 — Create a lane worktree for each PR phase `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/loop-repo-lane`
- **Depends on:** Phase 145

### Phase 121 — Find a loop session by its plan id `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/wsd-loop-plan-id`
- **Depends on:** Phase 144

### Phase 122 — Serve the current plan text of a loop from wsd `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/wsd-loop-plan-read`
- **Depends on:** Phase 121

### Phase 123 — Send the plan commands of loop-plan.sh to wsd for skills plans `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/loop-plan-on-wsd`
- **Depends on:** Phase 122

### Phase 124 — Keep skills loops off the loop-web daemon in loop-emit.sh `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/loop-emit-wsd-only`
- **Depends on:** Phase 140

### Phase 127 — Keep each plan on one engine `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/loop-one-engine-per-plan`
- **Depends on:** Phase 93

### Phase 97 — Add the loop orchestrator skill `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/skill-loop`
- **Depends on:** Phase 139, Phase 140, Phase 144, Phase 93

### Phase 143 — Add the gate explainer, the retrospective and the progress checkpoints to the loop skill `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/skill-loop-explainers`
- **Depends on:** Phase 97

### Phase 116 — Test the skills engine end to end `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `test/loop-skills-e2e`
- **Depends on:** Phase 143, Phase 123

### Phase 100 — Review milestone 6 `[status: todo]` `[kind: pr-review]` `[gate: merge]`
- **Repository:** `all`
- **Depends on:** Phase 116, Phase 124, Phase 127

## Milestone 7 — Devices sync through a hub, and assistants read sessions through remote MCP (roadmap phase 3, MVP)
- **Integration branch (rstagi/dotfile):** `feat/ws-hub`

### Phase 151 — Add the device key and the WS root CA `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/ws-device-identity`
- **Depends on:** Phase 100

### Phase 152 — Serve the hub with mTLS and a device registry `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/ws-hub-serve`
- **Depends on:** Phase 151

### Phase 153 — Pair a new device with a short code `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/ws-device-pairing`
- **Depends on:** Phase 152

### Phase 154 — Sync the session logs of each device to the hub `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/wsd-hub-sync`
- **Depends on:** Phase 152

### Phase 156 — Add the VPS install of the hub `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/ws-hub-deploy`
- **Depends on:** Phase 152

### Phase 155 — Show the sessions of every device `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/ws-hub-sessions`
- **Depends on:** Phase 154

### Phase 157 — Serve a read-only remote MCP with revocable tokens `[status: todo]`
- **Repository:** `rstagi/dotfile`
- **Suggested branch:** `feat/ws-hub-remote-mcp`
- **Depends on:** Phase 155

### Phase 158 — Review milestone 7 `[status: todo]` `[kind: pr-review]`
- **Repository:** `all`
- **Depends on:** Phase 153, Phase 156, Phase 157
