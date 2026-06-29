/**
 * Stylized SVG mockup of the Conclave MD desktop app — a placeholder until a
 * real screenshot is exported. Shows the macOS chrome (title bar dots),
 * the sidebar with workspaces, the case list, and a verdict pane mid-
 * deliberation. Inline SVG = zero requests, scales perfectly, respects
 * currentColor for theme switching.
 */
export function HeroMockup({ className }: { className?: string }) {
  return (
    <div
      className={[
        "relative aspect-[4/3] w-full border border-hairline bg-paper-elevated shadow-[0_30px_80px_-30px_rgb(0_0_0/0.18)] dark:shadow-[0_30px_80px_-30px_rgb(0_0_0/0.6)]",
        className ?? "",
      ].join(" ")}
      role="img"
      aria-label="Vista del app Conclave MD en pleno proceso de deliberación"
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 800 600"
        className="absolute inset-0 h-full w-full text-ink"
        fill="none"
      >
        {/* Title bar */}
        <rect x="0" y="0" width="800" height="36" fill="var(--color-paper-subtle)" />
        <line x1="0" y1="36" x2="800" y2="36" stroke="currentColor" strokeOpacity="0.08" />
        <circle cx="18" cy="18" r="5" fill="#FF5F57" />
        <circle cx="36" cy="18" r="5" fill="#FEBC2E" />
        <circle cx="54" cy="18" r="5" fill="#28C840" />
        <text
          x="400"
          y="22"
          textAnchor="middle"
          fontSize="10"
          fontFamily="JetBrains Mono, ui-monospace, monospace"
          fill="currentColor"
          fillOpacity="0.4"
        >
          Conclave MD — Cardiología
        </text>

        {/* Sidebar */}
        <rect x="0" y="36" width="200" height="564" fill="var(--color-paper)" />
        <line x1="200" y1="36" x2="200" y2="600" stroke="currentColor" strokeOpacity="0.06" />

        {/* Sidebar workspace items */}
        <g fontFamily="ui-sans-serif, system-ui" fontSize="11" fill="currentColor">
          <text x="20" y="74" fontSize="9" fillOpacity="0.5" fontFamily="JetBrains Mono, monospace" letterSpacing="2">
            WORKSPACES
          </text>
          <rect x="14" y="84" width="172" height="24" fill="currentColor" fillOpacity="0.06" />
          <circle cx="26" cy="96" r="2.5" fill="currentColor" />
          <text x="36" y="100" fillOpacity="0.95">Cardiología</text>

          <circle cx="26" cy="124" r="2.5" fill="currentColor" fillOpacity="0.4" />
          <text x="36" y="128" fillOpacity="0.6">Oncología</text>

          <circle cx="26" cy="148" r="2.5" fill="currentColor" fillOpacity="0.4" />
          <text x="36" y="152" fillOpacity="0.6">Urgencias</text>

          <text x="20" y="200" fontSize="9" fillOpacity="0.5" fontFamily="JetBrains Mono, monospace" letterSpacing="2">
            CASOS
          </text>
          <rect x="14" y="210" width="172" height="22" fill="currentColor" fillOpacity="0.03" />
          <text x="20" y="224" fillOpacity="0.85">FA paroxística — 68 a.</text>
          <text x="20" y="248" fillOpacity="0.6">IAMCEST inferior — 54</text>
          <text x="20" y="272" fillOpacity="0.6">ICC NYHA III — 71</text>
        </g>

        {/* Main pane background */}
        <rect x="200" y="36" width="600" height="564" fill="var(--color-paper-subtle)" />

        {/* Deliberation card */}
        <rect
          x="232"
          y="76"
          width="536"
          height="220"
          fill="var(--color-paper-elevated)"
          stroke="currentColor"
          strokeOpacity="0.08"
        />
        <g fontFamily="ui-sans-serif, system-ui" fill="currentColor">
          <text x="252" y="104" fontSize="9" fontFamily="JetBrains Mono, monospace" letterSpacing="2" fillOpacity="0.5">
            CASO · DELIBERACIÓN EN CURSO
          </text>
          <text x="252" y="132" fontSize="18" fontWeight="500">
            Fibrilación auricular paroxística en paciente de 68 años
          </text>
          <text x="252" y="156" fontSize="11" fillOpacity="0.65">
            Anticoagulación en CHA₂DS₂-VASc ≥ 2 sin antecedente hemorrágico.
          </text>
        </g>

        {/* Step pills */}
        <g fontFamily="JetBrains Mono, monospace" fontSize="9">
          <g transform="translate(252, 184)">
            <rect width="92" height="22" fill="currentColor" fillOpacity="0.08" />
            <text x="46" y="14" textAnchor="middle" fill="currentColor" fillOpacity="0.8" letterSpacing="1">
              01 BRIEFING ✓
            </text>
          </g>
          <g transform="translate(352, 184)">
            <rect width="92" height="22" fill="currentColor" fillOpacity="0.08" />
            <text x="46" y="14" textAnchor="middle" fill="currentColor" fillOpacity="0.8" letterSpacing="1">
              02 BORRADOR ✓
            </text>
          </g>
          <g transform="translate(452, 184)">
            <rect width="92" height="22" fill="var(--color-ink)" />
            <text x="46" y="14" textAnchor="middle" fill="var(--color-paper)" letterSpacing="1">
              03 CRÍTICA…
            </text>
          </g>
          <g transform="translate(552, 184)">
            <rect width="92" height="22" fill="currentColor" fillOpacity="0.04" />
            <text x="46" y="14" textAnchor="middle" fill="currentColor" fillOpacity="0.4" letterSpacing="1">
              04 VEREDICTO
            </text>
          </g>
        </g>

        {/* Progress dots animated via pulseDot in CSS */}
        <g transform="translate(252, 248)">
          <circle cx="0" cy="0" r="2" fill="currentColor" className="animate-[pulse-dot_1.4s_ease-in-out_infinite]" />
          <circle cx="10" cy="0" r="2" fill="currentColor" className="animate-[pulse-dot_1.4s_ease-in-out_infinite] [animation-delay:200ms]" />
          <circle cx="20" cy="0" r="2" fill="currentColor" className="animate-[pulse-dot_1.4s_ease-in-out_infinite] [animation-delay:400ms]" />
          <text x="36" y="3" fontSize="11" fill="currentColor" fillOpacity="0.6" fontFamily="ui-sans-serif, system-ui">
            Revisando el borrador contra las guías ESC 2024…
          </text>
        </g>

        {/* Verdict draft card */}
        <rect
          x="232"
          y="316"
          width="536"
          height="252"
          fill="var(--color-paper-elevated)"
          stroke="currentColor"
          strokeOpacity="0.08"
        />
        <g fontFamily="ui-sans-serif, system-ui" fill="currentColor">
          <text x="252" y="344" fontSize="9" fontFamily="JetBrains Mono, monospace" letterSpacing="2" fillOpacity="0.5">
            BORRADOR DE VEREDICTO
          </text>
          <text x="252" y="370" fontSize="13" fontWeight="500">
            Anticoagulación con DOAC (apixabán 5 mg / 12 h)
          </text>
          <text x="252" y="394" fontSize="11" fillOpacity="0.7">
            Recomendación principal · certeza: alta
          </text>

          {/* Citation rows */}
          <g fontSize="10.5" fillOpacity="0.7">
            <text x="252" y="430">
              <tspan fontFamily="JetBrains Mono, monospace" fillOpacity="0.45">§</tspan>
              <tspan dx="6">guia-esc-fa-2024.pdf · p. 47</tspan>
            </text>
            <text x="252" y="452">
              <tspan fontFamily="JetBrains Mono, monospace" fillOpacity="0.45">§</tspan>
              <tspan dx="6">protocolo-hospital-fa.docx · p. 3</tspan>
            </text>
            <text x="252" y="474">
              <tspan fontFamily="JetBrains Mono, monospace" fillOpacity="0.45">§</tspan>
              <tspan dx="6">aha-2023-update.pdf · p. 12</tspan>
            </text>
          </g>

          {/* Red flags */}
          <line x1="252" y1="500" x2="748" y2="500" stroke="currentColor" strokeOpacity="0.08" />
          <text x="252" y="524" fontSize="9" fontFamily="JetBrains Mono, monospace" letterSpacing="2" fill="var(--color-danger)" fillOpacity="0.85">
            BANDERAS ROJAS
          </text>
          <text x="252" y="546" fontSize="10.5" fillOpacity="0.7">
            · Ajustar dosis si ClCr &lt; 50 mL/min · Revisar interacciones (P-gp / CYP3A4)
          </text>
        </g>
      </svg>
    </div>
  );
}
