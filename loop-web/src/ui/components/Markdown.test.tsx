import { describe, it, expect } from "vitest";
import { parseMarkdown, parseInline } from "./Markdown.tsx";

describe("parseInline", () => {
  it("splits bold / code / link spans out of plain text", () => {
    const spans = parseInline("run `npm test` then see **the docs** at [here](https://x.io/y)");
    expect(spans).toEqual([
      { t: "text", v: "run " },
      { t: "code", v: "npm test" },
      { t: "text", v: " then see " },
      { t: "bold", v: "the docs" },
      { t: "text", v: " at " },
      { t: "link", v: "here", href: "https://x.io/y" },
    ]);
  });
  it("returns a single text span when there is no markup", () => {
    expect(parseInline("just words")).toEqual([{ t: "text", v: "just words" }]);
  });
  it("takes the earliest match when constructs are adjacent", () => {
    expect(parseInline("**a**`b`")).toEqual([
      { t: "bold", v: "a" },
      { t: "code", v: "b" },
    ]);
  });
});

describe("parseMarkdown", () => {
  it("parses paragraphs, joining soft-wrapped lines with a space", () => {
    const b = parseMarkdown("line one\nline two\n\nsecond para");
    expect(b).toEqual([
      { t: "p", spans: [{ t: "text", v: "line one line two" }] },
      { t: "p", spans: [{ t: "text", v: "second para" }] },
    ]);
  });
  it("groups consecutive '- ' / '* ' lines into one bullet list", () => {
    const b = parseMarkdown("- first\n* second");
    expect(b).toEqual([
      {
        t: "ul",
        items: [[{ t: "text", v: "first" }], [{ t: "text", v: "second" }]],
      },
    ]);
  });
  it("emits an unrecognized block (heading) verbatim as a raw block", () => {
    const b = parseMarkdown("## Findings\n\nprose");
    expect(b[0]).toEqual({ t: "raw", text: "## Findings" });
    expect(b[1]).toEqual({ t: "p", spans: [{ t: "text", v: "prose" }] });
  });
  it("keeps inline markup inside bullets", () => {
    const b = parseMarkdown("- do **this** and `that`");
    expect(b).toEqual([
      {
        t: "ul",
        items: [
          [
            { t: "text", v: "do " },
            { t: "bold", v: "this" },
            { t: "text", v: " and " },
            { t: "code", v: "that" },
          ],
        ],
      },
    ]);
  });
  it("returns an empty list for empty / blank input", () => {
    expect(parseMarkdown("")).toEqual([]);
    expect(parseMarkdown("   \n\n")).toEqual([]);
  });
});
