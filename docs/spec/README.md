# hoi design notes and specification

These are the design notes and the specification for a preview release. They describe intent and
may lag behind the code: where they differ, the Rust code in `core/`, the conformance vectors in
`core/vectors/` and the TypeScript contract in `packages/core-api/` are authoritative.

- [PROTOCOL.md](PROTOCOL.md): the wire-level protocol: records, kinds, stamps, identity work, sync messages, the ladder, review, media, content rules, Warmth and scattered delivery's formats.
- [SCATTER.md](SCATTER.md): scattered delivery: how posts are sealed, cut into pieces and placed so that no single node holds or hands over a whole post.
- [TECHNICAL.md](TECHNICAL.md): the technical overview: constraints, architecture, stack, platforms, crate layout, the validation pipeline and security.
- [scatter-params.mjs](scatter-params.mjs): the reference sizing for scattered delivery, in exact
  integer arithmetic. `node docs/spec/scatter-params.mjs` prints SCATTER.md's tables; with
  `--check core/vectors/scatter.json` it checks the vectors (part of `pnpm check:quick`).
