# RFC-0000: How hoi makes decisions

## Summary

Changes to hoi's protocol, storage, cryptography, architecture, governance or contributor rules
are made through RFCs: pull requests that add `rfcs/NNNN-short-title.md`. This process is
changed the same way.

## Process

- Maintainers are the members of the `hoi-project/maintainers` GitHub team.
- Anyone can open an RFC or comment on one. Discuss the problem in an issue first if you are not
  sure it needs one.
- A maintainer starts the comment period by commenting its end date on the pull request:
  **7 days** for protocol, storage, cryptography or governance changes and for changes to this
  process, **3 days** for everything else.
- After the period, the RFC merges with the approval of a maintainer other than its author, if
  no maintainer's objection is unresolved. An objection gives a reason and says what would
  resolve it. While hoi has one maintainer, that maintainer may merge their own RFC after the
  comment period, and says so on the pull request.
- A maintainer can decide an objection, or merge or close an RFC early, and says why on the
  pull request. Security fixes may be merged before the period ends and explained afterwards.
- An RFC is not edited after it merges, except for its status and links. A later RFC that
  changes the decision says which one it supersedes; the old one's status then names it.
- Status is one of: `proposed` (open pull request), `accepted` (merged), `superseded by
  RFC-NNNN`, `withdrawn`.

## Implementation

An accepted RFC is implemented in ordinary pull requests that cite it. Protocol changes also
update `docs/spec/PROTOCOL.md` and the conformance vectors in `core/vectors/`
([.claude/rules/protocol.md](../.claude/rules/protocol.md)).
