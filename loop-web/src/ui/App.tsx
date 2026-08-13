import { useEffect, useMemo, useState } from "react";
import type { Snapshot } from "../model/types.ts";
import { parsePlan, buildSnapshot } from "../model/index.ts";
import { useSnapshot } from "./hooks/useSnapshot.ts";
import { useLoopList } from "./hooks/useLoopList.ts";
import { pickDefaultRunId } from "./pick-default.ts";
import { Header } from "./components/Header.tsx";
import { ProblemsRail } from "./components/ProblemsRail.tsx";
import { Timeline } from "./components/Timeline.tsx";
import { Drawer } from "./components/Drawer.tsx";
import { PastePlan } from "./components/PastePlan.tsx";
import { GraphCanvas } from "./graph/GraphCanvas.tsx";

export function App() {
  const loops = useLoopList();
  const [runId, setRunId] = useState<string | null>(readRunId);
  const { snapshot: live, conn } = useSnapshot(runId);
  const [previewText, setPreviewText] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // Once the loop list is known, keep the current selection if still present, else the default.
  useEffect(() => {
    if (loops.length === 0) return;
    const next = pickDefaultRunId(loops, runId);
    if (next !== runId) setRunId(next);
  }, [loops, runId]);

  // Persist the selection in the URL so a reload / shared link reopens the same loop.
  useEffect(() => {
    const url = new URL(window.location.href);
    if (runId) url.searchParams.set("runId", runId);
    else url.searchParams.delete("runId");
    window.history.replaceState(null, "", url);
  }, [runId]);

  const preview = useMemo(() => (previewText != null ? buildPreview(previewText) : null), [previewText]);
  const active: Snapshot | null = preview ?? live;
  const selectedNode = active?.graph.nodes.find((n) => n.id === selectedId) ?? null;
  const dataRunId = previewText != null ? null : runId; // preview has no server-backed data

  return (
    <div className="app">
      <Header
        snapshot={active}
        conn={previewText != null ? "live" : conn}
        loops={loops}
        currentRunId={runId}
        onSelectRun={(id) => {
          setPreviewText(null);
          setSelectedId(null);
          setRunId(id);
        }}
      />
      <div className="stage">
        <ProblemsRail problems={active?.problems ?? []} onSelect={setSelectedId} />

        <div className="stage__center">
          {active ? (
            <GraphCanvas snapshot={active} selectedId={selectedId} onSelect={(n) => setSelectedId(n?.id ?? null)} />
          ) : (
            <div className="rail__empty" style={{ marginTop: "22vh" }}>
              <b>Waiting for the observer…</b>
              Connecting to the loop stream.
            </div>
          )}

          {previewText != null && <div className="banner">PREVIEW — pasted plan, no loop running</div>}

          <PastePlan
            previewing={previewText != null}
            onPreview={(md) => {
              setPreviewText(md);
              setSelectedId(null);
            }}
            onClear={() => setPreviewText(null)}
          />

          {selectedNode && (
            <Drawer
              node={selectedNode}
              pr={selectedNode.repository
                ? active?.effort.repositories.find((repository) => repository.slug === selectedNode.repository)?.pr ?? active?.effort.pr ?? null
                : active?.effort.pr ?? null}
              plan={active?.plan ?? null}
              runId={dataRunId}
              onClose={() => setSelectedId(null)}
            />
          )}
        </div>

        <Timeline events={active?.events ?? []} />
      </div>
    </div>
  );
}

function readRunId(): string | null {
  try {
    return new URL(window.location.href).searchParams.get("runId");
  } catch {
    return null;
  }
}

function buildPreview(markdown: string): Snapshot {
  const now = Date.now();
  try {
    return buildSnapshot(parsePlan(markdown), null, { now, nowIso: new Date(now).toISOString() });
  } catch {
    return buildSnapshot(parsePlan("# Unparseable plan — Multi-Phase Plan\n"), null, {
      now,
      nowIso: new Date(now).toISOString(),
    });
  }
}
