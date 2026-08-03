import { useMemo, useState } from "react";
import type { Snapshot } from "../model/types.ts";
import { parsePlan, buildSnapshot } from "../model/index.ts";
import { useSnapshot } from "./hooks/useSnapshot.ts";
import { Header } from "./components/Header.tsx";
import { ProblemsRail } from "./components/ProblemsRail.tsx";
import { Timeline } from "./components/Timeline.tsx";
import { Drawer } from "./components/Drawer.tsx";
import { PastePlan } from "./components/PastePlan.tsx";
import { GraphCanvas } from "./graph/GraphCanvas.tsx";

export function App() {
  const { snapshot: live, conn } = useSnapshot();
  const [previewText, setPreviewText] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const preview = useMemo(() => (previewText != null ? buildPreview(previewText) : null), [previewText]);
  const active: Snapshot | null = preview ?? live;
  const selectedNode = active?.graph.nodes.find((n) => n.id === selectedId) ?? null;

  return (
    <div className="app">
      <Header snapshot={active} conn={previewText != null ? "live" : conn} />
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
            <Drawer node={selectedNode} pr={active?.effort.pr ?? null} onClose={() => setSelectedId(null)} />
          )}
        </div>

        <Timeline events={active?.events ?? []} />
      </div>
    </div>
  );
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
