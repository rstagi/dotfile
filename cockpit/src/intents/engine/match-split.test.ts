import { expect, test } from "vitest";
import { buildClusterIndex } from "./match.ts";
import { splitAsk } from "./split.ts";
import { VerbLexicon } from "./text.ts";

test("matching joins restatements and keeps unrelated requests separate", () => {
  const index = buildClusterIndex([
    { key: "tests", members: ["check the CI tests"], usage: { "tool:test": 1 }, rank: 0 },
    { key: "docs", members: ["write the API documentation"], usage: { "tool:write": 1 }, rank: 1 },
  ]);
  expect(index.match("check CI tests", new Set())).toMatchObject({ kind: "join", key: "tests" });
  expect(index.match("refund the order", new Set(["tool:test"]))).toEqual({ kind: "new", ambiguous: false });
  expect(splitAsk("check the CI tests and write the API documentation", new VerbLexicon(), index, new Set())?.map((part) => part.key)).toEqual(["tests", "docs"]);
  expect(splitAsk("check the CI tests and refund the order", new VerbLexicon(), index, new Set())).toBeNull();
});
