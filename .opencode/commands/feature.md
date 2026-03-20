---
description: Create a new feature via SDLC-based multi-agent collaboration
agent: product-owner
---

# /feature - Create New Feature

This command executes an SDLC workflow with explicit subagent ownership per step.

## Roles

- Primary: **@product-owner**
- Subagents: **@pm, @team-lead, @designer, @developer, @qa**

## RACI-lite

| Step | Responsible | Accountable | Consulted |
|---|---|---|---|
| Scope definition | @product-owner, @pm | @product-owner | @team-lead, @designer |
| Technical/design planning | @team-lead, @designer | @team-lead | @pm, @qa |
| Delivery planning | @pm | @pm | @product-owner, @team-lead |
| Implementation | @developer | @team-lead | @qa, @designer |
| Verification | @qa | @qa | @developer, @team-lead |
| Final acceptance | @product-owner | @product-owner | @pm, @team-lead, @qa |

## Workflow Steps

### Step 1 — Scope and Success Criteria
- **Owner:** @product-owner + @pm
- **Inputs:** stakeholder request, constraints, context
- **Actions:** define scope, outcomes, acceptance criteria, non-goals
- **Outputs:** `docs/<feature_name>/README.md`
- **Done:** acceptance criteria are testable and approved for planning

### Step 2 — Technical and UX Plan
- **Owner:** @team-lead + @designer
- **Inputs:** approved scope
- **Actions:** architecture and UX approach, risks, milestones, quality gates
- **Outputs:** `implementation_plan.md`, `design_brief.md`, optional diagrams
- **Done:** approach is feasible and reviewable

### Step 3 — Delivery Plan and Alignment
- **Owner:** @pm
- **Inputs:** plan artifacts from Step 2
- **Actions:** sequence tasks, define handoffs, confirm schedule/risk handling
- **Outputs:** `delivery_plan.md`, `risk_register.md`
- **Done:** stakeholder confirms plan and priorities

### Step 4 — Implementation Increment
- **Owner:** @developer
- **Inputs:** approved implementation/design/delivery plans
- **Actions:** implement scope slice, add/update tests, run quality checks
- **Outputs:** code changes + test evidence + implementation notes
- **Done:** ready for QA + Team Lead review

### Step 5 — Verification and Review
- **Owner:** @qa + @team-lead
- **Inputs:** implementation increment
- **Actions:** execute verification strategy, review architecture/code quality, classify findings
- **Outputs:** test report, review feedback, blocker list
- **Done:** either accepted or sent back with actionable fixes

### Step 6 — Iteration Loop
- **Owner:** @developer + @qa (coordinated by @pm)
- **Inputs:** blocker list and feedback
- **Actions:** fix, retest, re-review
- **Outputs:** updated code and reports
- **Done:** no open blockers against acceptance criteria

### Step 7 — Product Acceptance and Final Report
- **Owner:** @product-owner + @pm
- **Inputs:** verified increment and risk summary
- **Actions:** final acceptance decision and stakeholder report
- **Outputs:** delivery summary with decisions and follow-ups
- **Done:** feature accepted or explicitly deferred with rationale
