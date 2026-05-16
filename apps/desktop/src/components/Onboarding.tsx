import { useState } from "react";

import { Button } from "./Button";
import { ipc } from "../lib/ipc";

export function Onboarding({
  disclaimer,
  onAccepted,
}: {
  disclaimer: string;
  onAccepted: () => void;
}) {
  const [busy, setBusy] = useState(false);

  const accept = async () => {
    setBusy(true);
    try {
      await ipc.acceptDisclaimer();
      onAccepted();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-bg/85 backdrop-blur">
      <div className="canvas-grain animate-in mx-4 max-w-xl rounded-2xl border border-border bg-bg-elevated p-7 shadow-soft">
        <div className="mb-5 flex items-center gap-3">
          <div className="grid h-10 w-10 place-content-center rounded-xl bg-accent text-bg text-lg font-semibold">
            C
          </div>
          <div>
            <div className="text-base font-semibold text-ink">
              Welcome to Conclave
            </div>
            <div className="text-[12px] text-ink-faint">
              Local-first clinical decision support · v0.1
            </div>
          </div>
        </div>

        <h2 className="mb-2 text-[15px] font-semibold text-ink">
          Before we start
        </h2>
        <p className="mb-3 text-[13px] leading-relaxed text-ink-dim">
          Conclave is an experimental clinical decision-support tool. It is{" "}
          <strong className="text-ink">not a medical device</strong> and does{" "}
          <strong className="text-ink">
            not replace the judgement of a qualified clinician
          </strong>
          . Outputs may be incomplete, biased or wrong. Always validate
          recommendations against primary sources and institutional protocols
          before acting on them.
        </p>

        <div className="mb-5 rounded-lg border border-border-subtle bg-bg p-4 text-[12px] leading-relaxed text-ink-subtle">
          {disclaimer}
        </div>

        <p className="mb-5 text-[12px] text-ink-faint">
          By continuing you acknowledge that you have read and understood the
          above and accept full responsibility for clinical decisions.
        </p>

        <div className="flex justify-end gap-2">
          <Button variant="primary" size="lg" onClick={accept} loading={busy}>
            I understand — continue
          </Button>
        </div>
      </div>
    </div>
  );
}
