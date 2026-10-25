import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const SKILL = path.join(import.meta.dirname, "..", ".claude", "skills", "visual-explainer");
const EXPLORE = path.join(SKILL, "scripts", "explore.mjs");
const EXAMPLE = path.join(SKILL, "examples", "explore-login.graph.json");
const { parseExplore } = await import(path.join(SKILL, "scripts", "explore-spec.mjs"));
const { renderExplore } = await import(EXPLORE);
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "visual-explainer-explore-"));
process.on("exit", () => fs.rmSync(TMP, { recursive: true, force: true }));

const parse = (graph) => parseExplore(JSON.stringify(graph));
const run = (...args) => spawnSync(process.execPath, [EXPLORE, ...args], { encoding: "utf8" });
const writeGraph = (name, graph) => {
  const file = path.join(TMP, name);
  fs.writeFileSync(file, typeof graph === "string" ? graph : JSON.stringify(graph));
  return file;
};

test("a graph without nodes fails, and so does text that is not JSON", () => {
  assert.equal(parse({ nodes: [] }).ok, false);
  assert.match(parseExplore("{").error, /not valid JSON/);
  assert.match(parseExplore("[]").error, /JSON object/);
});

test("near-miss kinds and directions are read kindly, and unknown ones get the default", () => {
  const out = parse({
    direction: "TD",
    nodes: [{ id: "a", label: "A", type: "db" }, { id: "b", label: "B", type: "constructor" }],
    edges: [{ from: "a", to: "b", kind: "event" }, { from: "b", to: "a", kind: "__proto__" }],
  });
  assert.equal(out.graph.direction, "down");
  assert.deepEqual(out.graph.nodes.map((n) => n.type), ["database", "service"]);
  assert.deepEqual(out.graph.edges.map((e) => e.kind), ["async", "flow"]);
});

test("small mistakes are dropped with a warning and the rest is drawn", () => {
  const out = parse({
    nodes: [
      { id: "g", label: "G", type: "group" },
      { id: "a", label: "A", group: "g" },
      { id: "a", label: "Again" },
      { id: "b", label: "B", group: "a" },
      { id: "c", label: "C", group: "c" },
      { label: "No id" },
    ],
    edges: [{ from: "a", to: "zzz" }, { source: "a", target: "b" }],
  });
  assert.equal(out.ok, true);
  assert.equal(out.graph.nodes.length, 4);
  assert.equal(out.graph.nodes.find((n) => n.id === "a").group, "g");
  assert.equal(out.graph.nodes.find((n) => n.id === "b").group, undefined);
  assert.equal(out.graph.nodes.find((n) => n.id === "c").group, undefined);
  assert.deepEqual(out.graph.edges.map((e) => e.id), ["a->b"]);
  assert.equal(out.warnings.length, 5);
});

test("a group cannot sit inside itself, directly or through another group", () => {
  const out = parse({
    nodes: [{ id: "x", label: "X", type: "group", group: "y" }, { id: "y", label: "Y", type: "group", group: "x" }],
  });
  assert.ok(out.graph.nodes.every((n) => n.group === undefined));
  assert.equal(out.warnings.length, 2);
});

test("a step names an id as written or as cleaned, and an unknown id is dropped", () => {
  const out = parse({
    nodes: [{ id: "Auth Service", label: "Auth" }, { id: "db", label: "DB" }],
    edges: [{ from: "Auth Service", to: "db" }],
    steps: [{ title: "One", focus: ["Auth Service", "Auth-Service", "db", "nope", "Auth-Service->db"] }],
  });
  assert.deepEqual(out.graph.nodes.map((n) => n.id), ["Auth-Service", "db"]);
  assert.deepEqual(out.graph.steps[0].focus, ["Auth-Service", "Auth-Service", "db", "Auth-Service->db"]);
});

test("a link keeps only http, https and plain paths", () => {
  const out = parse({
    nodes: [{
      id: "a", label: "A",
      links: [
        { label: "ok", href: "https://example.com" },
        { label: "path", href: "src/a.ts" },
        { label: "bad", href: "javascript:alert(1)" },
        { label: "bad2", href: " JavaScript:alert(1)" },
        { label: "data", href: "data:text/html,x" },
      ],
    }],
  });
  assert.deepEqual(out.graph.nodes[0].links.map((l) => l.label), ["ok", "path"]);
});

test("long text is cut with an ellipsis and the node limit is enforced", () => {
  const out = parse({ nodes: Array.from({ length: 200 }, (_, i) => ({ id: `n${i}`, label: "x".repeat(200) })) });
  assert.equal(out.graph.nodes.length, 150);
  assert.equal(out.graph.nodes[0].label.length, 80);
  assert.ok(out.graph.nodes[0].label.endsWith("…"));
});

test("the page escapes the title and keeps the data from ending its script", () => {
  const graph = parse({
    title: "A <b>&\"'",
    summary: "$& and </script>",
    nodes: [{ id: "a", label: "</script><img src=x>", details: "$1 $& $`" }],
  }).graph;
  const html = renderExplore(graph, { template: "<title>{{TITLE}}</title><p>{{SUMMARY}}</p><i>{{OUTLINE}}</i><script>{{DATA}}</script>", elk: "" });
  assert.ok(html.includes("<title>A &lt;b&gt;&amp;&quot;&#39;</title>"));
  assert.equal(html.match(/<\/script>/g).length, 1);
  assert.ok(!html.includes("<img"));
  assert.ok(html.includes("$1 $& $`"));
});

test("render writes one page that holds ELK, the data and the template", () => {
  const out = path.join(TMP, "out");
  const res = run("--graph", EXAMPLE, "--out", out);
  assert.equal(res.status, 0, res.stderr);
  const file = path.join(out, "explore.html");
  assert.equal(res.stdout.trim(), file);
  const html = fs.readFileSync(file, "utf8");
  assert.match(html, /<title>How login works<\/title>/);
  assert.ok(html.includes("EclipseLayoutKernel") || html.includes("elk"));
  assert.ok(!/\{\{[A-Z]+\}\}/.test(html), "a placeholder is left");
  assert.ok(!/<script[^>]+src=/.test(html), "the page loads no external script");
  assert.match(html, /prefers-color-scheme: dark/);
});

test("the CLI exits 0 for a valid graph, 1 for an invalid one and 2 for a usage error", () => {
  assert.equal(run("--graph", EXAMPLE, "--check").status, 0);
  const bad = run("--graph", writeGraph("bad.json", { nodes: [] }), "--check");
  assert.equal(bad.status, 1);
  assert.match(bad.stderr, /nodes/);
  assert.equal(run("--graph", EXAMPLE).status, 2);
  assert.equal(run("--graph", path.join(TMP, "missing.json"), "--check").status, 2);
});

test("the CLI prints each warning on stderr and still succeeds", () => {
  const res = run("--graph", writeGraph("warn.json", { nodes: [{ id: "a", label: "A" }], edges: [{ from: "a", to: "b" }] }), "--check");
  assert.equal(res.status, 0);
  assert.match(res.stderr, /warning: The line from "a" to "b"/);
});
