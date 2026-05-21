import { ImageResponse } from "next/og";

export const dynamic = "force-static";

export const alt = "Conclave — A virtual committee for clinical decision support";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

export default async function OpenGraphImage() {
  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          display: "flex",
          flexDirection: "column",
          justifyContent: "space-between",
          background: "#faf9f8",
          padding: "72px 80px",
          fontFamily: "'SF Pro Display', Inter, system-ui, sans-serif",
        }}
      >
        {/* Top row: brand mark + wordmark */}
        <div style={{ display: "flex", alignItems: "center", gap: 18 }}>
          <svg
            width="52"
            height="52"
            viewBox="0 0 64 64"
            fill="none"
            stroke="#121214"
            strokeWidth="5"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <circle cx="32" cy="32" r="24" />
            <circle cx="32" cy="8" r="5.5" fill="#121214" stroke="none" />
            <circle cx="9.17" cy="24.58" r="5.5" fill="#121214" stroke="none" />
            <circle cx="17.89" cy="51.42" r="5.5" fill="#121214" stroke="none" />
            <circle cx="46.11" cy="51.42" r="5.5" fill="#121214" stroke="none" />
            <circle cx="54.83" cy="24.58" r="5.5" fill="#121214" stroke="none" />
            <circle cx="32" cy="32" r="8" fill="#121214" stroke="none" />
          </svg>
          <div
            style={{
              fontFamily: "'JetBrains Mono', monospace",
              fontSize: 26,
              color: "#121214",
              letterSpacing: "-0.01em",
              fontWeight: 500,
            }}
          >
            Conclave
          </div>
        </div>

        {/* Headline */}
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: 28,
            maxWidth: 980,
          }}
        >
          <div
            style={{
              fontSize: 84,
              lineHeight: 0.96,
              letterSpacing: "-0.035em",
              color: "#121214",
              fontWeight: 500,
              display: "flex",
              flexDirection: "column",
            }}
          >
            <span>A virtual committee.</span>
            <span style={{ color: "#646469" }}>On your machine.</span>
          </div>
          <div
            style={{
              fontSize: 24,
              lineHeight: 1.4,
              color: "#3c3c40",
              maxWidth: 780,
            }}
          >
            An AI committee that argues against itself before answering. 100%
            local. No telemetry.
          </div>
        </div>

        {/* Bottom row: tags */}
        <div
          style={{
            display: "flex",
            gap: 24,
            fontFamily: "'JetBrains Mono', monospace",
            fontSize: 16,
            letterSpacing: "0.18em",
            textTransform: "uppercase",
            color: "#646469",
          }}
        >
          <span>Local-first</span>
          <span>·</span>
          <span>No telemetry</span>
          <span>·</span>
          <span>Not a medical device</span>
        </div>
      </div>
    ),
    { ...size },
  );
}
