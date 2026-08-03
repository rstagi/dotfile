import { useState } from "react";

/**
 * Preview an arbitrary plan markdown with no loop running. The pure model layer is
 * isomorphic (no fs/DOM), so the browser builds the static graph client-side — the server
 * stays a read-only file observer.
 */
export function PastePlan({
  previewing,
  onPreview,
  onClear,
}: {
  previewing: boolean;
  onPreview: (markdown: string) => void;
  onClear: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");

  if (!open) {
    return (
      <div className="paste">
        <button className="paste__btn" onClick={() => setOpen(true)}>
          ⧉ Paste plan
        </button>
      </div>
    );
  }
  return (
    <div className="paste">
      <div className="paste__panel">
        <div className="eyebrow" style={{ marginBottom: 6 }}>Preview a plan markdown</div>
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={"# My effort — Multi-Phase Plan\n\n## Phases\n\n### Phase 1 — Do the thing `[lane: A]` `[status: todo]`\n- **Depends on:** none"}
          spellCheck={false}
        />
        <div className="paste__row">
          <button className="btn btn--go" onClick={() => text.trim() && onPreview(text)}>
            Preview
          </button>
          {previewing && (
            <button className="btn" onClick={onClear}>
              Back to live
            </button>
          )}
          <button className="btn" style={{ marginLeft: "auto" }} onClick={() => setOpen(false)}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
