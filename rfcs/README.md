# RFCs

Decisions that change hoi's protocol, storage, cryptography, architecture, governance or
contributor rules are made in public, as RFCs: pull requests that add one file here,
`rfcs/NNNN-short-title.md`. The process itself is [RFC-0000](0000-decision-process.md).

Bugs and small improvements do not need an RFC; open an issue and a pull request
([CONTRIBUTING.md](../CONTRIBUTING.md)).

## Writing one

Copy this outline into `rfcs/NNNN-short-title.md`, with the next free number:

```markdown
# RFC-NNNN: <the decision, as a short statement>

Issue: #N

## Summary
One paragraph: what changes.

## Motivation
The problem, who it affects, what it costs today.

## Proposal
What changes, precisely enough to implement and review. For protocol changes: records, fields,
validation stages, compatibility and migration.

## Alternatives
Options considered and why they lost.

## Consequences
Compatibility, privacy and metadata exposed, who runs what, what becomes harder.
```

Open the pull request with the title `docs(rfc): RFC-NNNN <title>`. Implementation goes in
separate pull requests that cite the RFC (`RFC-NNNN` in comments and descriptions), after it is
merged.

## Index

| RFC                                   | Title                     | Status   |
| ------------------------------------- | ------------------------- | -------- |
| [0000](0000-decision-process.md)      | How hoi makes decisions   | accepted |
| [0006](0006-node-verification.md)    | Node declarations and verification | proposed |
