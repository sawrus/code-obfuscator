---
trigger: always_on
glob: sdlc-role-responsibilities
description: Role matrix for SDLC responsibilities, handoffs, and decision rights across subagents
---

# SDLC Role Responsibilities Matrix

## Roles

- `@product-owner`: value, scope, acceptance, prioritization.
- `@pm`: planning cadence, dependency/risk management, stakeholder communication.
- `@team-lead`: technical strategy, architecture quality, engineering sign-off.
- `@developer`: implementation and technical correctness.
- `@qa`: verification strategy and quality recommendation.
- `@designer`: UX quality, information architecture, interaction consistency.

## SDLC Phase Ownership

| SDLC Phase | Primary owner(s) | Key outputs |
|---|---|---|
| Requirements | @product-owner, @pm | Problem statement, acceptance criteria, scope decisions |
| Design | @team-lead, @designer | Implementation plan, UX/design brief, risk register |
| Implementation | @developer | Code changes, tests, implementation notes |
| Verification | @qa, @team-lead | Test report, defect log, review feedback |
| Deployment | @pm, @team-lead | Go/no-go decision, rollout/rollback plan |
| Maintenance | @developer, @qa, @team-lead | Incident fixes, postmortems, hardening backlog |

## Handoff Contracts

1. **Requirements -> Design**
   - include: acceptance criteria, constraints, non-goals.
2. **Design -> Implementation**
   - include: architecture boundaries, UX states, risk controls.
3. **Implementation -> Verification**
   - include: test evidence, known limitations, migration/release notes.
4. **Verification -> Acceptance/Release**
   - include: blocker status, residual risks, recommendation.

## Definition of Done (Cross-team)

- Acceptance criteria validated.
- No unresolved blocking defects.
- Required checks pass (lint/test/build/security as applicable).
- Documentation and operational notes updated.
