# 0003: Vertical wire connectivity is an open question until Phase 6

Status: approved by the project owner, 2026-11-01.

## Context

Section 8 does not say how wire connects between layers. The Phase 0 prototype
(`docs/phase0/circuits.md`) used six-direction connectivity, which merges stacked wire. Minecraft
uses step-up and step-down rules. Solid blocks carrying power and non-binary component outputs
were not prototyped either.

## Decision

Do not decide now. Record it as an open question in section 8 and section 17, and decide when
circuits can be played with in Phase 6. Whether the rules feel right is judged by building with
them, not by tests.

## Consequences

- Nothing before Phase 6 depends on the answer.
- The solver design must not assume six-direction connectivity in its public interface.
